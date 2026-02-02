use amplifier::encoder::Encoder;
use amplifier::stepper::Stepper;
use amplifier::mcp::Mcp;
use askama::Template;
use axum::response::sse::KeepAlive;
use mcp230xx::Mcp23017;
use mcp230xx;
use std::env;
use rppal::gpio::{Gpio, OutputPin};
use axum::response::{Html, IntoResponse, Redirect};
use axum::{
    Router,
    extract::{Multipart, Path, State},
    http::StatusCode,
    response::sse::{Event, Sse},
    routing::{get, post},
};
use axum_extra::TypedHeader;
use async_stream::stream;
use futures_util::stream::{self, Stream};
use futures_util::StreamExt;
use serde::{Deserialize, Serialize};
use serde_json;
use std::collections::HashMap;
use std::fs;
use std::io::Error;
use std::path;
use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex, mpsc};
use std::thread;
use std::{convert::Infallible, path::PathBuf, time::Duration};
use tokio::sync::broadcast::{self, Sender, Receiver};
use tokio::fs::File;
use tokio::io::{self, AsyncReadExt};
use tokio::process::Command;
use tokio::time::{interval, sleep, timeout};
use tokio_tungstenite::connect_async;
use tokio_tungstenite::tungstenite::Message;
use tokio_stream::StreamExt as TokioStreamExt;
use tower_http::{services::ServeDir, trace::TraceLayer};
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};
use chrono;
const ENABLE_PIN: u8 = 16;

#[derive(Template)]
#[template(path = "amplifier2.html")]
struct IndexTemplate<'a> {
    name: &'a str,
}
#[derive(Template)]
#[template(path = "config2.html")]
struct ConfigTemplate {
    enc: bool,
    enc_val: Vec<String>,
    tune: Vec<String>,
    ind: Vec<String>,
    load: Vec<String>,
    pins: Vec<u8>,
    files: Vec<String>,
    val: String,
    tci_server: String,
    follow_me: bool,
    tci_status: String,
    default_profile: String,
    cat_enabled: bool,
    cat_status: String,
    rigctld_host: String,
    rigctld_port: u16,
    rig_model_id: i32,
    rig_serial_device: String,
    rig_baud: u32,
    rig_civaddr: String,
    rig_extra_conf: String,
}

#[derive(Clone, Serialize, Deserialize)]
struct SseData {
    tune: u32,
    ind: u32,
    load: u32,
    max: HashMap<String, u32>,
    sw_pos: Option<Select>,
    band: Bands,
    ratio: HashMap<String, u8>,
    plate_v: u32,
    plate_a: u32,
    screen_a: u32,
    grid_a: u32,
    pwr_btns: HashMap<String, [String; 2]>,
    temperature: f64,
    call_sign: String,
    time: String,
    status: String,
    tci_status: String,
    cat_status: String,
}
impl SseData {
    fn new() -> SseData {
        SseData {
            tune: 0,
            ind: 0,
            load: 0,
            max: HashMap::from([
                ("tune".to_string(), 100000),
                ("ind".to_string(), 100000),
                ("load".to_string(), 100000),
            ]),
            sw_pos: None,
            band: Bands::M11,
            ratio: HashMap::from([
                ("tune".to_string(), 1),
                ("ind".to_string(), 1),
                ("load".to_string(), 1),
            ]),
            plate_v: 0,
            plate_a: 0,
            screen_a: 0,
            grid_a: 0,
            pwr_btns: HashMap::from([
                ("Blwr".to_string(), ["OFF".to_string(), "OFF".to_string()]),
                ("Fil".to_string(), ["OFF".to_string(), "OFF".to_string()]),
                ("HV".to_string(), ["OFF".to_string(), "OFF".to_string()]),
                ("Oper".to_string(), ["OFF".to_string(), "OFF".to_string()]),
            ]),
            temperature: 0.0,
            time: String::new(),
            call_sign: String::from("-----"),
            status: "Hello ALL BAND AMP".to_string(),
            tci_status: "DISCONNECTED".to_string(),
            cat_status: "DISCONNECTED".to_string(),
        }
    }
}
#[derive(Clone, Serialize, Deserialize, Debug)]
struct StoredData {
    tune: HashMap<String, u32>,
    ind: HashMap<String, u32>,
    load: HashMap<String, u32>,
    enc: HashMap<String, u32>,
    mem: HashMap<String, HashMap<String, u32>>,
    band: Bands,
    call_sign: String,
    #[serde(default)]
    mem_valid: HashMap<String, bool>,
    #[serde(default)]
    tci_server: String,
    #[serde(default)]
    follow_me: bool,
    #[serde(default)]
    cat_enabled: bool,
    #[serde(default)]
    cat_status: String,
    #[serde(default)]
    rigctld_host: String,
    #[serde(default)]
    rigctld_port: u16,
    #[serde(default)]
    rig_model_id: i32,
    #[serde(default)]
    rig_serial_device: String,
    #[serde(default)]
    rig_baud: u32,
    #[serde(default)]
    rig_civaddr: String,
    #[serde(default)]
    rig_extra_conf: String,
}
impl StoredData {
    fn new() -> Self {
        Self {
            tune: HashMap::new(),
            ind: HashMap::new(),
            load: HashMap::new(),
            enc: HashMap::new(),
            mem: HashMap::new(),
            band: Bands::M10,
            call_sign: String::from("-----"),
            mem_valid: HashMap::new(),
            tci_server: String::new(),
            follow_me: false,
            cat_enabled: false,
            cat_status: String::new(),
            rigctld_host: "127.0.0.1".to_string(),
            rigctld_port: 4532,
            rig_model_id: 0,
            rig_serial_device: String::new(),
            rig_baud: 0,
            rig_civaddr: String::new(),
            rig_extra_conf: String::new(),
        }
    }
}
#[derive(Clone)]
struct AppState {
    //event_sender: broadcast::Sender<SseData>,
    tune: Arc<Mutex<Stepper>>,
    ind: Arc<Mutex<Stepper>>,
    load: Arc<Mutex<Stepper>>,
    enc: Option<Encoder>,
    sw_pos: Option<Select>,
    band: Bands,
    gauges: Gauges,
    file: String,
    sleep: bool,
    enable_pin: Arc<Mutex<OutputPin>>,
    pwr_btns: PwrBtns,
    pwr_btns_state: HashMap<String, [String;2]>,
    temperature: f64,
    gpio_pins: Vec<u8>,
    call_sign: String,
    status: String,
    sender: Sender<String>,
    mem_valid: HashMap<String, bool>,
    tci_server: String,
    follow_me: bool,
    last_tci_band: Option<Bands>,
    tci_status: String,
    cat_enabled: bool,
    cat_status: String,
    rigctld_host: String,
    rigctld_port: u16,
    rig_model_id: i32,
    rig_serial_device: String,
    rig_baud: u32,
    rig_civaddr: String,
    rig_extra_conf: String,
    last_cat_band: Option<Bands>,
    default_profile: String,
    meter_sender: Option<mpsc::Sender<bool>>,
}
#[derive(Clone, Serialize, Deserialize)]
enum Select {
    Tune,
    Ind,
    Load,
}
#[derive(Clone, Serialize, Deserialize, Debug, PartialEq)]
enum Bands {
    M10,
    M11,
    M15,
    M20,
    M40,
    M80,
}
#[derive(Clone, Serialize, Deserialize)]
struct Gauges {
    plate_v: u32,
    plate_a: u32,
    screen_a: u32,
    grid_a: u32,
}
#[derive(Clone)]
struct PwrBtns {
    Blwr: [Mcp23017; 1],
    Fil: [Mcp23017; 2],
    HV: [Mcp23017; 2],
    Oper: [Mcp23017; 1],
    mcp: Mcp,
    bands: [Mcp23017; 5],
}
impl PwrBtns {
    fn new() -> Self {
        let mcp = Mcp::new();
        Self {
            Blwr: [*mcp.pins.get("A0").unwrap()],
            Fil: [*mcp.pins.get("A1").unwrap(), *mcp.pins.get("A2").unwrap()],
            HV: [*mcp.pins.get("A3").unwrap(), *mcp.pins.get("A4").unwrap()],
            Oper: [*mcp.pins.get("A5").unwrap()],
            bands: [*mcp.pins.get("B0").unwrap(),
                    *mcp.pins.get("B1").unwrap(),
                    *mcp.pins.get("B2").unwrap(),
                    *mcp.pins.get("B3").unwrap(),
                    *mcp.pins.get("B4").unwrap(),],
            mcp: {let mut output  = Mcp::new();
                output.init();
                output}

        }
    }
}

#[tokio::main]
async fn main() -> Result<(), std::io::Error> {
    let (tx, _rx) = broadcast::channel(1024);
    let app_state = Arc::new(Mutex::new(AppState {
        tune: Arc::new(Mutex::new(Stepper::new("tune"))),
        ind: Arc::new(Mutex::new(Stepper::new("ind"))),
        load: Arc::new(Mutex::new(Stepper::new("load"))),
        enc: None, //Some(Encoder::new(24, 23)),
        sw_pos: None,
        band: Bands::M10,
        gauges: Gauges {
            plate_v: 3000, //temporary for show
            plate_a: 1,
            screen_a: 50,
            grid_a: 10,
        },
        file: String::from("amplifier.json"),
        sleep: false,
        enable_pin: {
            let gpio = Gpio::new().unwrap();
            let mut pin = gpio.get(ENABLE_PIN).unwrap().into_output();
            pin.set_high();
            Arc::new(Mutex::new(pin))
        },
        pwr_btns : PwrBtns::new(),
        pwr_btns_state: HashMap::from([
                ("Blwr".to_string(), ["OFF".to_string(), "OFF".to_string()]),
                ("Fil".to_string(), ["OFF".to_string(), "OFF".to_string()]),
                ("HV".to_string(), ["OFF".to_string(), "OFF".to_string()]),
                ("Oper".to_string(), ["OFF".to_string(), "OFF".to_string()]),
            ]),
        temperature: 0.0,
        gpio_pins: vec![17, 27, 22, 5, 6, 13, 19,
                        26,14, 15, 18, 23, 24, 25,
                        12, 20, 21],
        call_sign: String::new(),
        status: String::new(),
        sender: tx,
        mem_valid: HashMap::from([
            ("10M".to_string(), false),
            ("11M".to_string(), false),
            ("15M".to_string(), false),
            ("20M".to_string(), false),
            ("40M".to_string(), false),
            ("80M".to_string(), false),
        ]),
        tci_server: String::new(),
        follow_me: false,
        last_tci_band: None,
        tci_status: "DISCONNECTED".to_string(),
        cat_enabled: false,
        cat_status: "DISCONNECTED".to_string(),
        rigctld_host: "127.0.0.1".to_string(),
        rigctld_port: 4532,
        rig_model_id: 0,
        rig_serial_device: String::new(),
        rig_baud: 0,
        rig_civaddr: String::new(),
        rig_extra_conf: String::new(),
        last_cat_band: None,
        default_profile: String::new(),
        meter_sender: None,
    }));
    {
        let (tx, _rx) = mpsc::channel();
        app_state.lock().unwrap().meter_sender = Some(tx);
    }
    if let Some(profile_name) = read_default_profile_name() {
        let _ = load_profile_from_file(app_state.clone(), &profile_name);
    }
    tokio::spawn(aquire_data(app_state.clone()));
    tokio::spawn(aquire_i2c_data(app_state.clone()));
    tokio::spawn(tci_follow_task(app_state.clone()));
    tokio::spawn(cat_follow_task(app_state.clone()));
    tracing_subscriber::registry()
        .with(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| {
                format!("{}=debug,tower_http=debug", env!("CARGO_CRATE_NAME")).into()
            }),
        )
        .with(tracing_subscriber::fmt::layer())
        .init();
    let listener = tokio::net::TcpListener::bind("0.0.0.0:3000").await.unwrap();
    tracing::debug!("listening on {}", listener.local_addr().unwrap());
    let assets_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("assets");
    let static_files_service = ServeDir::new(assets_dir).append_index_html_on_directories(true);
    // build our application with a route
    let app = Router::new()
        .fallback_service(static_files_service)
        .route("/sse", get(sse_handler))
        .route("/config", get(config_get).post(config_post))
        .route(
            "/",
            get(|| async {
                let template = IndexTemplate { name: "Axum User" };
                Html(template.render().unwrap())
            }),
        )
        //.route("/", get(default))
        .nest_service("/static", ServeDir::new("static"))
        .route("/selector/{val}", post(selector))
        .route("/store/{band}", post(store))
        .route("/recall/{band}", post(recall))
        .route("/stop", post(stop))
        .route("/load",  post(load))
        .route("/pwr_btn", post(pwr_btn_handler))
        .layer(TraceLayer::new_for_http())
        .with_state(app_state);
    let _ = axum::serve(listener, app).await;
    Ok(())
}

// receiver form data from config page.
async fn config_post(
    State(state): State<Arc<Mutex<AppState>>>,
    form: Multipart,
) -> impl IntoResponse {
    let form_data = process_form(form).await;
    let mut state = state.lock().unwrap();
    println!("FormData: {:?}", form_data);
    if form_data.contains_key("tci_server") || form_data.contains_key("follow_me") {
        if let Some(server) = form_data.get("tci_server") {
            let server = server.trim();
            if server.is_empty() {
                state.tci_server = String::new();
            } else if server.starts_with("ws://") || server.starts_with("wss://") {
                state.tci_server = server.to_string();
            } else {
                state.status = "Invalid TCI server URL (must start with ws:// or wss://)".to_string();
                return Redirect::to("/config");
            }
        }
        if let Some(follow) = form_data.get("follow_me") {
            state.follow_me = follow == "on";
        }
        state.status = format!(
            "TCI settings updated (Follow Me: {})",
            if state.follow_me { "ON" } else { "OFF" }
        );
    }
    if form_data.contains_key("save_cat")
        || form_data.contains_key("cat_enabled")
        || form_data.contains_key("rigctld_host")
        || form_data.contains_key("rigctld_port")
        || form_data.contains_key("rig_model_id")
        || form_data.contains_key("rig_serial_device")
        || form_data.contains_key("rig_baud")
        || form_data.contains_key("rig_civaddr")
        || form_data.contains_key("rig_extra_conf")
    {
        state.cat_enabled = form_data.get("cat_enabled").map(|v| v == "on").unwrap_or(false);
        if let Some(host) = form_data.get("rigctld_host") {
            state.rigctld_host = host.trim().to_string();
        }
        if let Some(port) = form_data.get("rigctld_port") {
            let port = port.trim();
            if port.is_empty() {
                state.rigctld_port = 4532;
            } else if let Ok(parsed) = port.parse::<u16>() {
                state.rigctld_port = parsed;
            } else {
                state.status = "Invalid rigctld port".to_string();
                return Redirect::to("/config");
            }
        }
        if let Some(model_id) = form_data.get("rig_model_id") {
            let model_id = model_id.trim();
            state.rig_model_id = model_id.parse::<i32>().unwrap_or(0);
        }
        if let Some(dev) = form_data.get("rig_serial_device") {
            state.rig_serial_device = dev.trim().to_string();
        }
        if let Some(baud) = form_data.get("rig_baud") {
            let baud = baud.trim();
            state.rig_baud = baud.parse::<u32>().unwrap_or(0);
        }
        if let Some(addr) = form_data.get("rig_civaddr") {
            state.rig_civaddr = addr.trim().to_string();
        }
        if let Some(extra) = form_data.get("rig_extra_conf") {
            state.rig_extra_conf = extra.trim().to_string();
        }
        state.status = format!(
            "CAT settings updated (Auto band: {})",
            if state.cat_enabled { "ON" } else { "OFF" }
        );
    }
    if state.cat_enabled && state.follow_me {
        state.follow_me = false;
        state.status = "CAT and TCI cannot both be enabled; CAT kept ON, TCI turned OFF".to_string();
    }

    if let Some(_) = state.enc  {
        if form_data.contains_key("del_enc") {
            let pin_a = state.enc.clone().unwrap().pin_a;
            let pin_b = state.enc.clone().unwrap().pin_b;
            let _ = process_pins(&mut state.gpio_pins, pin_a, false);
            let _ = process_pins(&mut state.gpio_pins, pin_b, false);
            *state.enc.clone().unwrap().stop.lock().unwrap() = true;
            state.enc = None;
            state.status = "Encoder has benn deleted!".to_string();
            
        }
        else if form_data.contains_key("add_tune") {
            if let Some(_) = state.tune.lock().unwrap().pin_a {
                println!("PinA already initialized for Tune");
            } else {
                handle_stepper(&mut state, form_data.clone(),  "Tune", true,|state| state.tune.clone());
                
            }
        }
        else if form_data.contains_key("del_tune") {
            handle_stepper(&mut state, form_data.clone(),  "Tune", false, |state| state.tune.clone()); 
        }
        else if form_data.contains_key("add_ind") {
            if let Some(_) = state.ind.lock().unwrap().pin_a {
                println!("PinA already initialized for Ind");
            } else {
                handle_stepper(&mut state, form_data.clone(),  "Ind", true,|state| state.ind.clone()); 
            }
        }
        else if form_data.contains_key("del_ind") {
            handle_stepper(&mut state, form_data.clone(),  "Ind", false ,|state| state.ind.clone()); 
        }
        else if form_data.contains_key("add_load") {
            if let Some(_) = state.load.lock().unwrap().pin_a {
                println!("PinA already initialized for Load");
            } else {
                handle_stepper(&mut state, form_data.clone(),  "Load", true,|state| state.load.clone()); 
                
            }
        }
        else if form_data.contains_key("del_load") {
            handle_stepper(&mut state, form_data.clone(),  "Load", false ,|state| state.load.clone()); 
            } 
        else if form_data.contains_key("start") {
            state.sw_pos = None;
            match form_data.get("start").unwrap().as_str() {
                "tune" => {
                    if let Some(tx) = state.meter_sender.clone() {
                        let _ = tx.send(false);
                    }
                    let state_tune = state.tune.lock().unwrap();
                    state_tune.pos.store(0, Ordering::Relaxed);
                }
                "ind" => {
                    if let Some(tx) = state.meter_sender.clone() {
                        let _ = tx.send(false);
                    }
                    let state_ind = state.ind.lock().unwrap();
                    state_ind.pos.store(0, Ordering::Relaxed);
                }
                "load" => {
                    if let Some(tx) = state.meter_sender.clone() {
                        let _ = tx.send(false);
                    }
                    let state_load = state.load.lock().unwrap();
                    state_load.pos.store(0, Ordering::Relaxed);
                }
                _ => println!("Invalid argument")
            }
        }  
        else if form_data.contains_key("max") {
            match form_data.get("max").unwrap().as_str() {
                "tune" => {
                    if let Some(tx) = state.meter_sender.clone() {
                        let _ = tx.send(false);
                    }
                    let state_tune = state.tune.lock().unwrap();
                    state_tune.max.store(state_tune.pos.load(Ordering::Relaxed), Ordering::Relaxed);
                }
                "ind" => {
                    if let Some(tx) = state.meter_sender.clone() {
                        let _ = tx.send(false);
                    }
                    let state_ind = state.ind.lock().unwrap();
                    state_ind.max.store(state_ind.pos.load(Ordering::Relaxed), Ordering::Relaxed);
                }
                "load" => {
                    if let Some(tx) = state.meter_sender.clone() {
                        let _ = tx.send(false);
                    }
                    let state_load = state.load.lock().unwrap();
                    state_load.max.store(state_load.pos.load(Ordering::Relaxed), Ordering::Relaxed);
                }
                _ => println!("Invalid argument") 
            }
            println!("Max was set");
        }  else if form_data.contains_key("reset") {
            match form_data.get("reset").unwrap().as_str() {
                "tune" => {
                    if let Some(tx) = state.meter_sender.clone() {
                        let _ = tx.send(false);
                    }
                    let state_tune = state.tune.lock().unwrap();
                    state_tune.max.store(100000, Ordering::Relaxed);
                }
                "ind" => {
                    if let Some(tx) = state.meter_sender.clone() {
                        let _ = tx.send(false);
                    }
                    let state_ind = state.ind.lock().unwrap();
                    state_ind.max.store(100000, Ordering::Relaxed);
                }
                "load" => {
                    if let Some(tx) = state.meter_sender.clone() {
                        let _ = tx.send(false);
                    }
                    let state_load = state.load.lock().unwrap();
                    state_load.max.store(100000, Ordering::Relaxed);
                }
                _ => println!("Invalid argument")
            }
        }
    } else {
        if form_data.contains_key("PinA") && form_data.contains_key("PinB") {
                if form_data.get("PinA").unwrap() != "" && form_data.get("PinB").unwrap() != "" {
                let pin_a = form_data.get("PinA").unwrap().parse().unwrap();
                let pin_b = form_data.get("PinB").unwrap().parse().unwrap();
                state.enc = Some(Encoder::new(
                    pin_a,
                    pin_b,
                ));
                let _ = state.enc.clone().unwrap().run();
                let _ = process_pins(&mut state.gpio_pins, form_data.get("PinA").unwrap().parse().unwrap(), true);
                let _ = process_pins(&mut state.gpio_pins, form_data.get("PinB").unwrap().parse().unwrap(), true);
                println!("Encoder Added");
                state.status = format!(
                    "Encoder Added on pins: {:?}, {:?}",
                    form_data.get("PinA").unwrap(),
                    form_data.get("PinB").unwrap(),
                );
            }
        }
    }
    if form_data.clone().contains_key("call_sign") {
        state.call_sign = form_data.get("call_sign").unwrap().clone();
        println!("Callsign added: {}", state.call_sign);
    }
    Redirect::to("/config")
}

fn process_pins(pin_list: &mut Vec<u8>, val: u8, remove: bool) -> Result<(), Box< dyn std::error::Error>> {
    if remove {
        if let Some(out) = pin_list.iter().position(|&x| x == val) {
            pin_list.remove(out);
            return Ok(())
        } else {
            return Err(Box::new(Error::new(io::ErrorKind::Other, "Pin not Found")))
        }
    } else {
        pin_list.push(val);
        return Ok(())
    }
  
}
// Route handler for GET request for config page.
async fn config_get(State(state): State<Arc<Mutex<AppState>>>) -> Html<String> {
    println!("Config get was called.");
    let state = state.lock().unwrap();
    let tune = state.tune.lock().unwrap();
    let ind = state.ind.lock().unwrap();
    let load = state.load.lock().unwrap();
    let template = ConfigTemplate {
        enc: if let Some(_) = state.enc { true } else { false },
        enc_val: if let Some(_) = state.enc {
            vec![
                state.enc.clone().unwrap().pin_a.to_string(),
                state.enc.clone().unwrap().pin_b.to_string(),
            ]
        } else {
            vec!["None".to_string(), "None".to_string()]
        },
        tune: if let Some(_) = tune.pin_a {
            vec![
                tune.pin_a.unwrap().to_string(),
                tune.pin_b.unwrap().to_string(),
                tune.ratio.to_string(),
            ]
        } else {
            vec!["None".to_string(), "None".to_string(), 1.to_string()]
        },
        ind: if let Some(_) = ind.pin_a {
            vec![
                ind.pin_a.unwrap().to_string(),
                ind.pin_b.unwrap().to_string(),
                ind.ratio.to_string(),
            ]
        } else {
            vec!["None".to_string(), "None".to_string(), 1.to_string()]
        },
        load: if let Some(_) = load.pin_a {
            vec![
                load.pin_a.unwrap().to_string(),
                load.pin_b.unwrap().to_string(),
                load.ratio.to_string(),
            ]
        } else {
            vec!["None".to_string(), "None".to_string(), 1.to_string()]
        },
        files: {
            let home_path = env::current_dir().unwrap().join("static");
            let mut output: Vec<String> = Vec::new();
            let files =
                fs::read_dir(home_path).unwrap();
            files.for_each(|f| {
                let temp_file = f.unwrap().file_name().to_string_lossy().to_string();
                if temp_file.ends_with("json") {
                    output.push(temp_file);
                }
            }); 
            output
        },
        val: "TEST".to_string(),
        pins: state.gpio_pins.clone(),
        tci_server: state.tci_server.clone(),
        follow_me: state.follow_me,
        tci_status: state.tci_status.clone(),
        default_profile: state.default_profile.clone(),
        cat_enabled: state.cat_enabled,
        cat_status: state.cat_status.clone(),
        rigctld_host: state.rigctld_host.clone(),
        rigctld_port: state.rigctld_port,
        rig_model_id: state.rig_model_id,
        rig_serial_device: state.rig_serial_device.clone(),
        rig_baud: state.rig_baud,
        rig_civaddr: state.rig_civaddr.clone(),
        rig_extra_conf: state.rig_extra_conf.clone(),
    };
    Html(template.render().unwrap().to_string())
}
// Processes initial SSE Request (Route Handler).
async fn sse_handler(
    TypedHeader(_): TypedHeader<headers::UserAgent>,
    State(app_state): State<Arc<Mutex<AppState>>>,
) -> Sse<impl Stream<Item = Result<Event, Infallible>>> {
    let state_lck = app_state.lock().unwrap();
    let mut rx = state_lck.sender.subscribe();
    Sse::new(stream! {
        while let Ok(msg) = rx.recv().await {
            yield Ok(Event::default().data::<String>(msg));
        }
    }).keep_alive(KeepAlive::default())

}

fn split_frames(s: &str) -> impl Iterator<Item = &str> {
    s.split(|c| c == ';' || c == '\n' || c == '\r')
        .map(str::trim)
        .filter(|f| !f.is_empty())
}

fn parse_any_tx_hz(frame: &str) -> Option<u64> {
    let (k, v) = frame.split_once(':')?;
    let k = k.trim();

    if k == "tx_frequency" || k == "rx_frequency" || k == "trx_frequency" {
        return v.trim().parse::<u64>().ok();
    }

    if k == "tx_frequency_thetis" || k == "rx_frequency_thetis" || k == "trx_frequency_thetis" {
        let mut parts = v.split(',').map(str::trim);
        let hz: u64 = parts.next()?.parse().ok()?;
        return Some(hz);
    }

    None
}

fn band_from_hz(hz: u64) -> Option<Bands> {
    match hz {
        3_500_000..=4_000_000 => Some(Bands::M80),
        7_000_000..=7_300_000 => Some(Bands::M40),
        14_000_000..=14_350_000 => Some(Bands::M20),
        21_000_000..=21_450_000 => Some(Bands::M15),
        26_000_000..=27_999_999 => Some(Bands::M11),
        28_000_000..=29_700_000 => Some(Bands::M10),
        _ => None,
    }
}

fn band_to_key(band: &Bands) -> &'static str {
    match band {
        Bands::M10 => "10M",
        Bands::M11 => "11M",
        Bands::M15 => "15M",
        Bands::M20 => "20M",
        Bands::M40 => "40M",
        Bands::M80 => "80M",
    }
}

async fn tci_follow_task(state: Arc<Mutex<AppState>>) {
    let mut active_server = String::new();
    loop {
        let (server, enabled, cat_enabled) = {
            let state_lck = state.lock().unwrap();
            (state_lck.tci_server.clone(), state_lck.follow_me, state_lck.cat_enabled)
        };

        if cat_enabled {
            {
                let mut state_lck = state.lock().unwrap();
                state_lck.tci_status = "PAUSED".to_string();
            }
            sleep(Duration::from_millis(500)).await;
            continue;
        }

        if !enabled || server.is_empty() {
            {
                let mut state_lck = state.lock().unwrap();
                state_lck.tci_status = "DISCONNECTED".to_string();
            }
            sleep(Duration::from_millis(500)).await;
            continue;
        }

        active_server = server.clone();
        {
            let mut state_lck = state.lock().unwrap();
            state_lck.tci_status = "CONNECTING".to_string();
            state_lck.status = format!("TCI connecting: {}", active_server);
        }
        println!("TCI: connecting to {}", active_server);
        match connect_async(active_server.as_str()).await {
            Ok((mut ws, _)) => {
                {
                    let mut state_lck = state.lock().unwrap();
                    state_lck.tci_status = "CONNECTED".to_string();
                    state_lck.status = format!("TCI connected: {}", active_server);
                }
                println!("TCI: connected to {}", active_server);
                loop {
                    tokio::select! {
                        msg = futures_util::StreamExt::next(&mut ws) => {
                            match msg {
                                Some(Ok(Message::Text(s))) => {
                                    for frame in split_frames(&s) {
                                        if let Some(hz) = parse_any_tx_hz(frame) {
                                            if let Some(band) = band_from_hz(hz) {
                                                println!("TCI: band {} at {} Hz", band_to_key(&band), hz);
                                                {
                                                    let mut state_lck = state.lock().unwrap();
                                                    state_lck.status = format!(
                                                        "Follow Me: detected {} at {} Hz",
                                                        band_to_key(&band),
                                                        hz
                                                    );
                                                }
                                                let maybe_recall = {
                                                    let mut state_lck = state.lock().unwrap();
                                                    if !state_lck.follow_me || state_lck.tci_server != active_server {
                                                        None
                                                    } else if state_lck.last_tci_band == Some(band.clone()) {
                                                        None
                                                    } else {
                                                        state_lck.last_tci_band = Some(band.clone());
                                                        let tune_busy = *state_lck.tune.lock().unwrap().operate.lock().unwrap();
                                                        let ind_busy = *state_lck.ind.lock().unwrap().operate.lock().unwrap();
                                                        let load_busy = *state_lck.load.lock().unwrap().operate.lock().unwrap();
                                                        if tune_busy || ind_busy || load_busy {
                                                            state_lck.status = "Follow Me: tune in progress, skipping".to_string();
                                                            None
                                                        } else if state_lck.band == band {
                                                            None
                                                        } else {
                                                            Some((band.clone(), band_to_key(&band)))
                                                        }
                                                    }
                                                };

                                                if let Some((band_enum, band_key)) = maybe_recall {
                                                    match recall_handler(state.clone(), band_key.to_string(), band_enum, true) {
                                                        Ok(_) => println!("TCI: recall {}", band_key),
                                                        Err(e) => println!("TCI: recall {} failed: {}", band_key, e),
                                                    }
                                                }
                                            }
                                        }
                                    }
                                }
                                Some(Ok(Message::Close(_))) | None => break,
                                _ => {}
                            }
                        }
                        _ = sleep(Duration::from_millis(250)) => {
                            let state_lck = state.lock().unwrap();
                            if !state_lck.follow_me || state_lck.tci_server != active_server {
                                break;
                            }
                        }
                    }
                }
                let mut state_lck = state.lock().unwrap();
                state_lck.tci_status = "DISCONNECTED".to_string();
                state_lck.status = format!("TCI disconnected: {}", active_server);
            }
            Err(_) => {
                {
                    let mut state_lck = state.lock().unwrap();
                    state_lck.status = format!("TCI connect failed: {}", active_server);
                    state_lck.tci_status = "ERROR".to_string();
                }
                println!("TCI: connect failed to {}", active_server);
                sleep(Duration::from_secs(2)).await;
            }
        }
    }
}

async fn cat_follow_task(state: Arc<Mutex<AppState>>) {
    let mut cat_connected = false;
    loop {
        let (enabled, model_id, device, baud, civaddr, extra_conf) = {
            let state_lck = state.lock().unwrap();
            (
                state_lck.cat_enabled,
                state_lck.rig_model_id,
                state_lck.rig_serial_device.clone(),
                state_lck.rig_baud,
                state_lck.rig_civaddr.clone(),
                state_lck.rig_extra_conf.clone(),
            )
        };

        if !enabled {
            {
                let mut state_lck = state.lock().unwrap();
                state_lck.cat_status = "DISCONNECTED".to_string();
            }
            cat_connected = false;
            sleep(Duration::from_millis(500)).await;
            continue;
        }

        if model_id == 0 || device.is_empty() {
            {
                let mut state_lck = state.lock().unwrap();
                state_lck.cat_status = "ERROR".to_string();
                state_lck.status = "CAT enabled but model/device not set".to_string();
            }
            cat_connected = false;
            sleep(Duration::from_millis(500)).await;
            continue;
        }

        if !cat_connected {
            let mut state_lck = state.lock().unwrap();
            state_lck.cat_status = "POLLING".to_string();
        }

        let mut cmd = Command::new("rigctl");
        cmd.arg("-m")
            .arg(model_id.to_string())
            .arg("-r")
            .arg(device.clone());
        if baud != 0 {
            cmd.arg("-s").arg(baud.to_string());
        }
        if !civaddr.trim().is_empty() {
            cmd.arg("-c").arg(civaddr.trim());
        }
        if !extra_conf.trim().is_empty() {
            cmd.arg("-C").arg(extra_conf.trim());
        }
        cmd.arg("f");

        let output = timeout(Duration::from_millis(1200), cmd.output()).await;
        match output {
            Ok(Ok(out)) => {
                let stdout = String::from_utf8_lossy(&out.stdout);
                let line = stdout.lines().next().unwrap_or("").trim();
                if line.starts_with("RPRT") || line.is_empty() {
                    let mut state_lck = state.lock().unwrap();
                    state_lck.cat_status = "ERROR".to_string();
                    state_lck.status = format!("CAT error: {}", line);
                    cat_connected = false;
                } else if let Ok(hz) = line.parse::<u64>() {
                    if let Some(band) = band_from_hz(hz) {
                        let maybe_recall = {
                            let mut state_lck = state.lock().unwrap();
                            if !cat_connected {
                                state_lck.cat_status = "CONNECTED".to_string();
                            }
                            if !state_lck.cat_enabled {
                                None
                            } else if state_lck.last_cat_band == Some(band.clone()) {
                                None
                            } else {
                                state_lck.last_cat_band = Some(band.clone());
                                let tune_busy = *state_lck.tune.lock().unwrap().operate.lock().unwrap();
                                let ind_busy = *state_lck.ind.lock().unwrap().operate.lock().unwrap();
                                let load_busy = *state_lck.load.lock().unwrap().operate.lock().unwrap();
                                if tune_busy || ind_busy || load_busy {
                                    state_lck.status = "CAT: tune in progress, skipping".to_string();
                                    None
                                } else if state_lck.band == band {
                                    None
                                } else {
                                    Some((band.clone(), band_to_key(&band)))
                                }
                            }
                        };
                        if let Some((band_enum, band_key)) = maybe_recall {
                            if let Err(e) = recall_handler(state.clone(), band_key.to_string(), band_enum, true) {
                                let mut state_lck = state.lock().unwrap();
                                state_lck.status = format!("CAT recall {} failed: {}", band_key, e);
                            }
                        }
                    }
                    cat_connected = true;
                } else {
                    let mut state_lck = state.lock().unwrap();
                    state_lck.cat_status = "ERROR".to_string();
                    state_lck.status = format!("CAT parse error: {}", line);
                    cat_connected = false;
                }
            }
            Ok(Err(e)) => {
                let mut state_lck = state.lock().unwrap();
                state_lck.cat_status = "ERROR".to_string();
                state_lck.status = format!("CAT rigctl failed: {}", e);
                cat_connected = false;
            }
            Err(_) => {
                let mut state_lck = state.lock().unwrap();
                state_lck.cat_status = "ERROR".to_string();
                state_lck.status = "CAT rigctl timeout".to_string();
                cat_connected = false;
            }
        }
        sleep(Duration::from_millis(400)).await;
    }
}

//Selects a stepper to be tuned.
async fn selector(
    Path(val): Path<String>, State(app_state): State<Arc<Mutex<AppState>>>,
    mut form_data: Multipart,
) -> impl IntoResponse {
    println!("Form handler");
    println!("{}", val);
    app_state.lock().unwrap().enable_pin.lock().unwrap().set_low();
    let state_lck = app_state.lock().unwrap().clone();
    let tune = state_lck.tune.lock().unwrap().clone();
    let ind = state_lck.ind.lock().unwrap().clone();
    let load = state_lck.load.lock().unwrap().clone();
    if  *tune.operate.lock().unwrap() == false && *ind.operate.lock().unwrap() == false && *load.operate.lock().unwrap() == false {
        while let Some(val) = form_data.next_field().await.unwrap() {
            println!("Name: {}", val.name().unwrap().to_string());
            match val.name().unwrap() {
                "tune" => {
                    let mut state = app_state.lock().unwrap();
                    if let Ok(_) = selector_handler(&mut state, |x| x.tune.clone()) {
                        state.status = "Tune is selected".to_string();
                        state.sw_pos = Some(Select::Tune);
                    }
                }
                "ind" => {
                    let mut state = app_state.lock().unwrap();
                    if let Ok(_) = selector_handler(&mut state, |x| x.ind.clone()) {
                        state.status = "Ind is selected".to_string();
                        state.sw_pos = Some(Select::Ind);
                        
                        
                    }
                }
                "load" => {
                    let mut state = app_state.lock().unwrap();
                    if let Ok(_) = selector_handler(&mut state, |x| x.load.clone()) {
                        state.status = "Load is selected".to_string();
                        state.sw_pos = Some(Select::Load);
                    }
                    
                }
                _ => {
                    println!("Invalid form Entry");
                }
            }
        }
    } else {
        app_state.lock().unwrap().status = format!("Cannot select a tuner while tune is in progress ! ! !");
    }
    StatusCode::OK
}

fn selector_handler<F>(state: &mut AppState,  callback: F) -> Result<(), Box<dyn std::error::Error>>
where F:
        Fn(&mut AppState) -> Arc<Mutex<Stepper>> {
    if let Some(tx) = state.meter_sender.clone() {
        let _ = tx.send(false);
    }
    let stepper = callback(state);
    if let Some(enc) = state.clone().enc {
        enc.count.store(stepper.clone().lock().unwrap().pos.load(Ordering::Relaxed), Ordering::Relaxed);
        return Ok(())
    } else {
        state.status = format!("No Encoder present! ! !");
        Err(Box::new(Error::new(std::io::ErrorKind::Other, "No Encoder Forund")))
        
    }

}
//Recalls bands from memory.
async fn recall(Path(path): Path<String>, State(state): State<Arc<Mutex<AppState>>>) {
    println!("{}", path);
    let state_lck = state.lock().unwrap().clone();
        if *state_lck.tune.lock().unwrap().operate.lock().unwrap() == false && *state_lck.ind.lock().unwrap().operate.lock().unwrap() == false && *state_lck.load.lock().unwrap().operate.lock().unwrap() == false  {
            state.lock().unwrap().sleep = true;
            match path.as_str() {
                "M10" => {
                    if let Ok(_) = recall_handler(state.clone(), "10M".to_string(), Bands::M10, false) {
                        
                    }
                }
                "M11" => {
                    if let Ok(_) = recall_handler(state.clone(), "11M".to_string(), Bands::M11, false) {
        
                    }
                }
                "M15" => {
                    if let Ok(_) = recall_handler(state.clone(), "15M".to_string(), Bands::M15, false) {

                    }
                }
                "M20" => {
                    if let Ok(_) = recall_handler(state.clone(), "20M".to_string(), Bands::M20, false) {
            
                    }
                }
                "M40" => {
                    if let Ok(_) = recall_handler(state.clone(), "40M".to_string(), Bands::M40, false) {
        
                    }
                }
                "M80" => {
                    if let Ok(_) = recall_handler(state.clone(), "80M".to_string(), Bands::M80, false) {
                    }
                }
                _ => {
                    println!("Invalid band selected!!")
                }
            }
        } else {
        state.lock().unwrap().status = format!("Attempted to recall while motors still in motion!!");
    }
}
// Saves data to JSON file from AppState.
async fn store(Path(path): Path<String>, State(state): State<Arc<Mutex<AppState>>>) {
    println!("Store Called");
    println!("{}", path);
    match path.as_str() {
        "M10" => {
            store_handler(state, "10M".to_string());
        }
        "M11" => {
            store_handler(state, "11M".to_string());
        }
        "M15" => {
            store_handler(state, "15M".to_string());
        }
        "M20" => {
            store_handler(state, "20M".to_string());
        }
        "M40" => {
            store_handler(state, "40M".to_string());
        }
        "M80" => {
            store_handler(state, "80M".to_string());
        }
        _ => {
            println!("Invalid band selected!!")
        }
    }
}

async fn stop(State(state): State<Arc<Mutex<AppState>>>) {
    println!("Save stop request received");
    sleep_save(state);

}
// Loads data from config file and initialized AppState.
async fn load(State(state): State<Arc<Mutex<AppState>>>, form: Multipart) ->
    impl IntoResponse {
    println!("Config PostForm Handler");
    let mut form_data = process_form(form).await;
    if form_data.contains_key("clear_default") {
        let _ = clear_default_profile_name();
        let mut state_lck = state.lock().unwrap();
        state_lck.default_profile = String::new();
        state_lck.status = "Default profile cleared".to_string();
    } else if form_data.contains_key("files") && form_data.contains_key("load") {
        let file_name = form_data.get("files").unwrap();
        println!("Filename: {}", file_name);
        let _ = load_profile_from_file(state.clone(), file_name);
        if form_data.contains_key("default_profile") {
            let _ = write_default_profile_name(file_name);
            let mut state_lck = state.lock().unwrap();
            state_lck.default_profile = file_name.to_string();
            state_lck.status = format!("Default profile set: {}", file_name);
        }
    } else if form_data.contains_key("file_name") {
            let mut file_name = form_data.get("file_name").unwrap().clone().to_string();
            file_name.push_str(".json");
            state.lock().unwrap().file = file_name.clone();
            state.lock().unwrap().status = format!("Saved data to: {}", file_name);
            println!("{}", file_name);
            println!("New file saved");
            sleep_save(state);
        }
    return Redirect::to("/config");
}

//power button handler.
async fn pwr_btn_handler(State(state): State<Arc<Mutex<AppState>>>, form: Multipart) {
    let form_data = process_form(form).await;
    if form_data.contains_key("ID") {
        let sw = form_data.get("ID").unwrap();
        println!("Switch: {}", sw);
        let action = form_data.get("value").unwrap();
        println!("Action: {}", action);
        match sw.as_str() {
            "Blwr" => {
                let mut state_lck = state.lock().unwrap();
                let pin = state_lck.pwr_btns.Blwr[0];
                let _ = state_lck.pwr_btns.mcp.set_pin(pin, if action == "ON" {mcp230xx::Level::High} else {mcp230xx::Level::Low}).unwrap_or(());
                state_lck.status = format!("{}", if action == "ON" {"Blower ON"} else {"Blower OFF"});

            }
            "Fil" => {
                step_start(&mut state.lock().unwrap(), form_data,"Filament".to_string(), |x| x.pwr_btns.Fil);
            }
            "HV" => {
                step_start(&mut state.lock().unwrap(), form_data,"HV".to_string(), |x| x.pwr_btns.HV);
                
            }
            "Oper" => {
                let mut state_lck = state.lock().unwrap();
                let pin = state_lck.pwr_btns.Oper[0];
                let _ = state_lck.pwr_btns.mcp.set_pin(pin, if action == "ON" {mcp230xx::Level::High} else {mcp230xx::Level::Low});
                state_lck.status = format!("{}", if action == "ON" {"Operate"} else {"Standby"});

            }

            _ => println!("Invalid selection of swithes")
        }
    }
}

//step start helper function
fn step_start<F>(state_lck: &mut AppState, form_data: HashMap<String, String>, name: String, callback: F)
where
    F: Fn(&mut AppState) -> [Mcp23017;2],
    {
        let action = form_data.get("value").unwrap();
        let my_btns = callback(state_lck);
        let pin1 = my_btns[0];
        let pin2 = my_btns[1];
        let pin1_status = state_lck.pwr_btns.mcp.read_pin(pin1).unwrap();
        let _ = state_lck.pwr_btns.mcp.set_pin(pin1, if action == "ON" {mcp230xx::Level::High} else {mcp230xx::Level::Low});  
        if form_data.contains_key("delay") {
            let delay = form_data.get("delay").unwrap();
            let _ = state_lck.pwr_btns.mcp.set_pin(pin2, if delay == "ON"  && pin1_status == mcp230xx::Level::High {mcp230xx::Level::High} else {mcp230xx::Level::Low});
            state_lck.status = format!("{}", if action == "ON" && delay == "OFF" {
                format!("{} Step Start !!!",  name)
            } else if pin1_status == mcp230xx::Level::High && delay == "ON" {
                format!("{}  ON ! ! !", name)
            } else {
                format!("{} Shutting Down...", name)
            });
        } 
    }
    
// Aquires data from peripheral devices and feeds SSE via a broadcast channel.
async fn aquire_data(state: Arc<Mutex<AppState>>) {
    let mut interval = interval(Duration::from_millis(10));
    println!("Aquire data");
    let mut count = 0;
    loop {
        interval.tick().await;
        let date_time = chrono::offset::Local::now().format("%m-%d-%Y, %H:%M:%S").to_string();
        let call_sign = state.lock().unwrap().call_sign.clone();
        let val = state.lock().unwrap().clone();
        let tune = val.tune.lock().unwrap().clone();
        let ind = val.ind.lock().unwrap().clone();
        let load = val.load.lock().unwrap().clone();
        if *tune.operate.lock().unwrap() == false && *ind.operate.lock().unwrap() == false && *load.operate.lock().unwrap() == false && val.sleep == true {
            count += 1;
            if count >= 10 {
                sleep_save(state.clone());
                count = 0;
            }
        } else {
            count = 0;
        }
        if let Some(_) = val.enc {
            let clone = val.enc.clone().unwrap().enc();
            if clone >= 0 {
                match val.sw_pos {
                    Some(Select::Tune) => {
                        let tune_max = tune.max.load(Ordering::Relaxed).saturating_sub(1);
                        if  clone < tune_max && clone > 0 {
                            if let Some(_) = tune.pin_a {
                                if let Some(ch) = tune.channel.clone() {
                                    let _ = ch.send((clone as u32, false));
                                }
                            } else {
                                tune.pos.store(clone, Ordering::Relaxed);
                            }
                        }
                    }
                    Some(Select::Ind) => {
                        let ind_max = ind.max.load(Ordering::Relaxed).saturating_sub(1);
                        if  clone < ind_max && clone > 0 {
                            if let Some(_) = ind.pin_a {
                                if let Some(ch) = ind.channel.clone() {
                                    let _ = ch.send((clone as u32, false));
                                }
                            } else {
                                ind.pos.store(clone, Ordering::Relaxed);
                            }
                        }
                    }
                    Some(Select::Load) => {
                        let load_max = load.max.load(Ordering::Relaxed).saturating_sub(1);
                        if  clone < load_max && clone > 0 {
                            if let Some(_) = load.pin_a {
                                if let Some(ch) = load.channel.clone() {
                                    let _ = ch.send((clone as u32, false));
                                }
                            } else {
                                load.pos.store(clone, Ordering::Relaxed);
                            }
                        }
                    }
                    None => {}
                }
            } else {
                val.enc.clone().unwrap().count.store(0, Ordering::Relaxed);
            }
        }
        let mut sse_output = SseData::new();
        sse_output.time = date_time;
        sse_output.call_sign = call_sign;
        sse_output.tune = tune.pos.load(Ordering::Relaxed) as u32;
        sse_output.ind = ind.pos.load(Ordering::Relaxed) as u32;
        sse_output.load = load.pos.load(Ordering::Relaxed) as u32;
        sse_output.sw_pos = val.sw_pos.clone();
        sse_output.band = val.band.clone();
        sse_output.max.entry("tune".to_string()).insert_entry(tune.max.load(Ordering::Relaxed) as u32);
        sse_output.max.entry("ind".to_string()).insert_entry(ind.max.load(Ordering::Relaxed) as u32);
        sse_output.max.entry("load".to_string()).insert_entry(load.max.load(Ordering::Relaxed) as u32);
        let temp_bands = HashMap::from([
            ("tune".to_string(), tune.ratio),
            ("ind".to_string(), ind.ratio),
            ("load".to_string(), load.ratio),
        ]);
        for (key, val) in temp_bands {
            sse_output.ratio.entry(key).insert_entry(val);
        }
        sse_output.pwr_btns = val.pwr_btns_state;
        sse_output.plate_v = val.gauges.plate_v;
        sse_output.plate_a = val.gauges.plate_a;
        sse_output.screen_a = val.gauges.screen_a;
        sse_output.grid_a = val.gauges.grid_a;
        sse_output.temperature = val.temperature;
        sse_output.status = val.status.clone();
        sse_output.tci_status = val.tci_status.clone();
        sse_output.cat_status = val.cat_status.clone();
        let _ = val.sender.send(serde_json::to_string(&sse_output).unwrap());    
    }
}

//aquires I2C data and loads it to the AppState global Mutex.
async fn aquire_i2c_data(state: Arc<Mutex<AppState>>) {
    let mut interval = interval(Duration::from_millis(100));
    let mut temp_data: HashMap<String, [String;2]> = HashMap::new();
    let (tx, rx) = mpsc::channel();
    state.lock().unwrap().meter_sender = Some(tx);
    let mut run = true;
    loop {
        interval.tick().await;
        let mut val = state.lock().unwrap().pwr_btns.clone();
        let btn_arr = [val.Blwr[0], val.Fil[0], val.Fil[1], val.HV[0], val.HV[1]];
        btn_arr.iter().enumerate().for_each(|btn|{
            if let Ok(val) = val.mcp.read_pin(*btn.1) {
                match btn.0 {
                    0 => {
                        temp_data.insert("Blwr".to_string(), [
                        if val == mcp230xx::Level::High {"ON".to_string()} else {"OFF".to_string()},
                        "OFF".to_string()]);
                    },
                    1 | 2 => {
                        temp_data.insert("Fil".to_string(), [
                        if val == mcp230xx::Level::High {"ON".to_string()} else {"OFF".to_string()},
                        if val == mcp230xx::Level::High {"ON".to_string()} else {"OFF".to_string()}]);
                    }
                    3 | 4 => {
                        temp_data.insert("HV".to_string(), [
                        if val == mcp230xx::Level::High {"ON".to_string()} else {"OFF".to_string()},
                        if val == mcp230xx::Level::High {"ON".to_string()} else {"OFF".to_string()}]);
                    
                    }
                    _ => println!("Match statement error with MCP Pins")

                }
                    
            } 
        });
        if let Ok(val) = rx.try_recv() {
            run = val;
        }
        let mut temp = 0.0;
        let mut screen_ma = 0_u32;
        let mut plate_v = 0_u32;
        if run {
            if let Ok(t)=  val.mcp.read_val() {
                plate_v = t.2 as u32;
                screen_ma = t.1 as u32;
                temp = t.0;
            } 
        } 
        let mut state_lck = state.lock().unwrap();
        state_lck.pwr_btns_state = temp_data.clone();
        state_lck.temperature = temp;
        state_lck.gauges.screen_a = screen_ma;
        state_lck.gauges.plate_v = plate_v as u32 * 100;
    }
        
}

//assistant function to create and initialize stepper motors
fn handle_stepper<F> (state: &mut AppState, form_data: HashMap<String, String>, name: &str, add: bool, process: F)
where
    F: Fn(&mut AppState) -> Arc<Mutex<Stepper>>,
    
 {
    let stepper = process(state);
    let mut state_stepper = stepper.lock().unwrap();
    if add {
        state.sw_pos = None;
        if form_data.get("PinA").unwrap() != "" && form_data.get("PinB").unwrap() != "" {
            println!("Adding Stepper");
            let pin_a: u8 = form_data.get("PinA").unwrap().parse().unwrap();
            let pin_b: u8 = form_data.get("PinB").unwrap().parse().unwrap();
            let ratio: u8 = form_data.get("ratio").unwrap().parse().unwrap_or(1);
            state_stepper.name = name.to_string().to_lowercase();
            state_stepper.pin_a = Some(pin_a);
            state_stepper.pin_b = Some(pin_b);
            state_stepper.ratio = ratio;
            let _ = process_pins(&mut state.gpio_pins, pin_a, true);
            let _ = process_pins(&mut state.gpio_pins, pin_b, true);
            if name == "Ind" {
                state_stepper.speed = Duration::from_micros(400);
            }
            state_stepper.run_2();
        } else {
            println!("No pins Selected");
        }
    } else {
        println!("Resetting {} to default settings", name
    );
        if let Some(_) = state_stepper.pin_a {
            println!("Deleting {}", state_stepper.name);
            let pin_a = state_stepper.pin_a.unwrap();
            let pin_b = state_stepper.pin_b.unwrap();
            let _ = process_pins(&mut state.gpio_pins, pin_a, false);
            let _ = process_pins(&mut state.gpio_pins, pin_b, false);
            let _ = state_stepper.channel.clone().unwrap().send((state_stepper.pos.load(Ordering::Relaxed) as u32, true));
            state_stepper.pin_a = None;
            state_stepper.pin_b = None;
            state_stepper.ratio = 1;
        }
    }
    let pina = state_stepper.pin_a.unwrap_or(0);
    let pinb = state_stepper.pin_b.unwrap_or(0);
    let ratio = state_stepper.ratio;
    let name: String = state_stepper.name.clone().to_lowercase();
    drop(state_stepper);
    state.status = {
        if add {
            format!("{} Added on Pins: {}, {}, ratio of {}",name, pina, pinb, ratio)
        } else {
            format!("{} Deleted...", name)
        }
    }
        
 }
// Assistand function for recall route.
fn recall_handler (state: Arc<Mutex<AppState>>, band: String, band_enum: Bands, require_stored: bool) -> Result<(), Box< dyn std::error::Error>> {
    let mut state_lck = state.lock().unwrap();
    if require_stored {
        if state_lck
            .mem_valid
            .get(&band)
            .copied()
            .unwrap_or(false)
            == false
        {
            state_lck.status = format!("No stored settings for {} band", band);
            if band_enum == Bands::M11 {
                state_lck.band = band_enum.clone();
                return Ok(());
            }
            return Err(Box::new(Error::new(std::io::ErrorKind::Other, "Band not stored")));
        }
    }
    if let Some(_) = state_lck.enc {
        if let Some(tx) = state_lck.meter_sender.clone() {
            let _ = tx.send(false);
        }
        state_lck.pwr_btns.clone().bands.iter().for_each(|pin|{
            let _ = state_lck.pwr_btns.clone().mcp.set_pin(*pin, mcp230xx::Level::Low);
        });
        match band_enum {
            Bands::M10 => {let _ = state_lck.pwr_btns.clone().mcp.set_pin(state_lck.pwr_btns.clone().bands[0], mcp230xx::Level::High);},
            Bands::M11 => {let _ = state_lck.pwr_btns.clone().mcp.set_pin(state_lck.pwr_btns.clone().bands[1], mcp230xx::Level::High);},
            Bands::M20 => {let _ = state_lck.pwr_btns.clone().mcp.set_pin(state_lck.pwr_btns.clone().bands[2], mcp230xx::Level::High);},
            Bands::M40 => {let _ = state_lck.pwr_btns.clone().mcp.set_pin(state_lck.pwr_btns.clone().bands[3], mcp230xx::Level::High);},
            Bands::M80 => {let _ = state_lck.pwr_btns.clone().mcp.set_pin(state_lck.pwr_btns.clone().bands[4], mcp230xx::Level::High);},
            Bands::M15 => {}
        }
        state_lck.band = band_enum;
        state_lck.sw_pos = None;
        state_lck.sleep = true;
        state_lck.enable_pin.lock().unwrap().set_low();
        let my_locks = [
            state_lck.tune.clone(),
            state_lck.ind.clone(),
            state_lck.load.clone(),
        ];
        if state_lck.enable_pin.lock().unwrap().is_set_low() {
            drop(state_lck);
            for x in my_locks {
                let value = band.clone();
                thread::spawn(move || {
                    let temp_lck = x.lock().unwrap().clone();
                    if let Some(_) = temp_lck.pin_a { 
                        let _ = temp_lck.channel.unwrap().send((temp_lck.mem.get(&value).unwrap().load(Ordering::Relaxed) as u32, false));
                    } else {
                        temp_lck.pos.store(temp_lck.mem.get(&value).unwrap().load(Ordering::Relaxed), Ordering::Relaxed);
                    }
                    println!("Run thread ended");

                });
                
            }
            let mut state_lck = state.lock().unwrap();
            state_lck.status = format!("Recalled {} Band ! ! !", band);
        } else {
            state_lck.status = format!("Error with enable pin!");
        }
    return Ok(())
    } else {
        Err(Box::new(Error::new(std::io::ErrorKind::Other, "No Encoder Present")))
    }
}
fn store_handler(state: Arc<Mutex<AppState>>, band: String) {
    let mut state_lck = state.lock().unwrap();
    let my_locks = [
        state_lck.tune.clone(),
        state_lck.ind.clone(),
        state_lck.load.clone(),
    ];
    for lock in my_locks {
        let value = band.clone();
        let mut stepper = lock.lock().unwrap();
        let pos = stepper.pos.load(Ordering::Relaxed);
        stepper.mem.entry(value).and_modify(|v| v.store(pos,Ordering::Relaxed));
    }
    state_lck.mem_valid.insert(band.clone(), true);
    state_lck.status = format!("Stored {} Band", band);

}
//funtion that stores all data when either save is presssed or after recall has been completed.
fn sleep_save(state: Arc<Mutex<AppState>>) {
    let mut state_lck = state.lock().unwrap();
    state_lck.sleep = false;
    println!("Sleep is: {}", state_lck.sleep);
    state_lck.enable_pin.lock().unwrap().set_high();
    println!("Sleep_Save Ran");
    state_lck.sw_pos = None;
    let file_path = path::Path::new(&state_lck.file);
    let dir = env::current_dir().unwrap();
    let full_path = dir.join("static").join(file_path);
    if !fs::exists(&full_path).unwrap() {
        let _ = fs::File::create(&full_path);
    }
    let mut saved_state = StoredData::new();
    if let Some(enc) = state_lck.clone().enc {
        saved_state.enc.entry("PinA".to_string()).insert_entry(enc.pin_a as u32);
        saved_state.enc.entry("PinB".to_string()).insert_entry(enc.pin_b as u32);
    } else {
        state_lck.status = "Encoder not configured; skipping encoder save".to_string();
    }
    saved_state.mem.entry("tune".to_string()).insert_entry(store_data_creator(&mut state_lck.clone(), &mut saved_state.tune, |x| x.tune.clone()));
    saved_state.mem.entry("ind".to_string()).insert_entry(store_data_creator(&mut state_lck.clone(), &mut saved_state.ind, |x| x.ind.clone()));
    saved_state.mem.entry("load".to_string()).insert_entry(store_data_creator(&mut state_lck.clone(), &mut saved_state.load, |x| x.load.clone()));
    saved_state.band = state_lck.band.clone();
    saved_state.call_sign = state_lck.call_sign.clone();
    saved_state.mem_valid = state_lck.mem_valid.clone();
    saved_state.tci_server = state_lck.tci_server.clone();
    saved_state.follow_me = state_lck.follow_me;
    saved_state.cat_enabled = state_lck.cat_enabled;
    saved_state.cat_status = state_lck.cat_status.clone();
    saved_state.rigctld_host = state_lck.rigctld_host.clone();
    saved_state.rigctld_port = state_lck.rigctld_port;
    saved_state.rig_model_id = state_lck.rig_model_id;
    saved_state.rig_serial_device = state_lck.rig_serial_device.clone();
    saved_state.rig_baud = state_lck.rig_baud;
    saved_state.rig_civaddr = state_lck.rig_civaddr.clone();
    saved_state.rig_extra_conf = state_lck.rig_extra_conf.clone();
    println!("Attempting to save data");
    if let Ok(output_data) = serde_json::to_string_pretty(&saved_state) {
        println!("Saving file to {}", full_path.to_string_lossy().to_string());
        if let Ok(_) = fs::write(full_path, output_data) {
            state_lck.status = format!("All data successfully saved !");
            if let Some(tx) = state_lck.meter_sender.clone() {
                let _ = tx.send(true);
            }
        }
    }
    
}
//Assistant function to store route
fn store_data_creator<F>(state_lck: &mut AppState, data: &mut HashMap<String,u32>, callback: F) -> HashMap<String, u32>
where
    F: Fn (&mut AppState) -> Arc<Mutex<Stepper>>,
    {
    let stepper = callback(state_lck);
    if let Some(pin_a) = stepper.lock().unwrap().pin_a {
        data.entry("PinA".to_string()).insert_entry(pin_a as u32);
        
    }
    if let Some(pin_b) = stepper.lock().unwrap().pin_b {
        data.entry("PinB".to_string()).insert_entry(pin_b as u32);

    }
    if let Some(ena) = stepper.lock().unwrap().ena {
        data.entry("ena".to_string()).insert_entry(ena as u32);

    }
    data.entry("ratio".to_string()).insert_entry(stepper.lock().unwrap().ratio as u32);
    data.entry("max".to_string()).insert_entry(stepper.lock().unwrap().max.load(Ordering::Relaxed) as u32);
    data.entry("pos".to_string()).insert_entry(stepper.lock().unwrap().pos.load(Ordering::Relaxed).clone() as u32);
    let mut temp_mem_data = HashMap::new();
    for (k, v) in stepper.lock().unwrap().mem.clone() {
        temp_mem_data.entry(k).insert_entry(v.load(Ordering::Relaxed)as u32);
        
    }
    temp_mem_data
    
    }

fn default_profile_path() -> PathBuf {
    env::current_dir()
        .unwrap()
        .join("static")
        .join("default_profile.txt")
}

fn read_default_profile_name() -> Option<String> {
    let path = default_profile_path();
    if let Ok(contents) = fs::read_to_string(path) {
        let name = contents.trim().to_string();
        if !name.is_empty() {
            return Some(name);
        }
    }
    None
}

fn write_default_profile_name(file_name: &str) -> Result<(), std::io::Error> {
    fs::write(default_profile_path(), format!("{}\n", file_name))
}

fn clear_default_profile_name() -> Result<(), std::io::Error> {
    let path = default_profile_path();
    if path.exists() {
        fs::remove_file(path)?;
    }
    Ok(())
}

fn load_profile_from_file(state: Arc<Mutex<AppState>>, file_name: &str) -> Result<(), String> {
    let full_path = env::current_dir()
        .map_err(|e| e.to_string())?
        .join("static")
        .join(file_name);
    let file_data = fs::read_to_string(full_path).map_err(|e| e.to_string())?;
    let output: StoredData = serde_json::from_str(&file_data).map_err(|e| e.to_string())?;
    apply_profile_to_state(state, file_name, output);
    Ok(())
}

fn apply_profile_to_state(state: Arc<Mutex<AppState>>, file_name: &str, output: StoredData) {
    let mut state_lck = state.lock().unwrap();
    state_lck.file = file_name.to_string();
    let mut my_stepper_arr = [
        state_lck.tune.clone(),
        state_lck.ind.clone(),
        state_lck.load.clone(),
    ];
    let bands = ["10M", "11M", "15M", "20M", "40M", "80M"];
    let my_output_arr = [&output.tune, &output.ind, &output.load];
    for (i, stepper) in my_stepper_arr.iter_mut().enumerate() {
        let name = &stepper.lock().unwrap().name.clone();
        if stepper.lock().unwrap().pin_a.unwrap_or(0u8) != 0 {
            handle_stepper(&mut state_lck, HashMap::new(), name, false, |x| stepper.clone());
        }
        thread::sleep(Duration::from_millis(10));
        stepper.lock().unwrap().pin_a = if my_output_arr[i].contains_key("PinA") {Some(*my_output_arr[i].get("PinA").unwrap() as u8)} else {None};
        stepper.lock().unwrap().pin_b = if my_output_arr[i].contains_key("PinB") {Some(*my_output_arr[i].get("PinB").unwrap() as u8)} else {None};
        stepper.lock().unwrap().ena = if my_output_arr[i].contains_key("ena") {Some(*my_output_arr[i].get("ena").unwrap() as u8)} else {None};
        stepper.lock().unwrap().max.store(*my_output_arr[i].get("max").unwrap() as i32, Ordering::Relaxed);
        stepper.lock().unwrap().pos.store(*my_output_arr[i].get("pos").unwrap() as i32, Ordering::Relaxed);
        stepper.lock().unwrap().ratio = *my_output_arr[i].get("ratio").unwrap() as u8;
        let mut stepper_lck = stepper.lock().unwrap();
        if stepper_lck.name == "ind" {
            println!("Inductor set to lower speed");
            stepper_lck.speed = Duration::from_micros(400);
        }
        if let Some(_) = stepper_lck.pin_a {
            stepper_lck.run_2();
        }
        drop(stepper_lck);
        for band in bands {
            let mut stepper_lck = stepper.lock().unwrap();
            let value = *output.mem.get(&stepper_lck.name).unwrap().get(&band.to_string()).unwrap_or(&0) as i32;
            stepper_lck.mem.entry(band.to_string()).and_modify(|v| v.store(value, Ordering::Relaxed));
        }
    }
    state_lck.enc = if output.enc.contains_key("PinA") && output.enc.contains_key("PinB") {
        if let Some(enc) = &state_lck.enc {
            *enc.stop.lock().unwrap() = true;
            println!("Deconfiguring Encoder to load new config");
        }
        Some(Encoder::new(
            *output.enc.get("PinA").unwrap() as u8,
            *output.enc.get("PinB").unwrap() as u8,
        ))
    } else {
        None
    };
    if let Some(mut enc) = state_lck.enc.clone() {
        let _ = enc.run();
    }
    state_lck.band = output.band;
    state_lck.call_sign = output.call_sign;
    let mut derived: HashMap<String, bool> = HashMap::new();
    for band in ["10M", "11M", "15M", "20M", "40M", "80M"] {
        let has_key =
            output.mem.get("tune").and_then(|m| m.get(band)).is_some()
            || output.mem.get("ind").and_then(|m| m.get(band)).is_some()
            || output.mem.get("load").and_then(|m| m.get(band)).is_some();
        let file_flag = output.mem_valid.get(band).copied().unwrap_or(false);
        derived.insert(band.to_string(), has_key || file_flag);
    }
    state_lck.mem_valid = derived;
    for key in ["10M", "11M", "15M", "20M", "40M", "80M"] {
        state_lck.mem_valid.entry(key.to_string()).or_insert(false);
    }
    if !output.tci_server.is_empty() {
        state_lck.tci_server = output.tci_server;
    }
    state_lck.follow_me = output.follow_me;
    state_lck.cat_enabled = output.cat_enabled;
    if state_lck.cat_enabled && state_lck.follow_me {
        state_lck.follow_me = false;
    }
    if !output.cat_status.is_empty() {
        state_lck.cat_status = output.cat_status;
    }
    if !output.rigctld_host.is_empty() {
        state_lck.rigctld_host = output.rigctld_host;
    }
    if output.rigctld_port != 0 {
        state_lck.rigctld_port = output.rigctld_port;
    }
    if output.rig_model_id != 0 {
        state_lck.rig_model_id = output.rig_model_id;
    }
    if !output.rig_serial_device.is_empty() {
        state_lck.rig_serial_device = output.rig_serial_device;
    }
    if output.rig_baud != 0 {
        state_lck.rig_baud = output.rig_baud;
    }
    if !output.rig_civaddr.is_empty() {
        state_lck.rig_civaddr = output.rig_civaddr;
    }
    if !output.rig_extra_conf.is_empty() {
        state_lck.rig_extra_conf = output.rig_extra_conf;
    }
}

async fn read_html_from_file<P: AsRef<path::Path>>(path: P) -> Result<String, std::io::Error> {
    let mut file = File::open(path).await?;
    let mut contents = String::new();
    file.read_to_string(&mut contents).await?;
    Ok(contents)
}
//processes all Multi-part form data for all post request handlers.
async fn process_form(mut form: Multipart) -> HashMap<String, String> {
    let mut form_data: HashMap<String, String> = HashMap::new();
    println!("Config PostForm Handler");
    while let Some(val) = form.next_field().await.unwrap() {
        println!("Name: {:?}", val.name().unwrap().to_string());
        let k = val.name().unwrap().to_string();
        let v = val.text().await.unwrap().to_string();
        println!("Key: {}, Value: {}", k, v);
        form_data.insert(k.clone(), v.clone());
    }
    println!("Pwr Button form data {:?}", form_data);
    form_data
}
