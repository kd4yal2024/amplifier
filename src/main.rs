use amplifier::encoder::Encoder;
use amplifier::stepper::Stepper;
use amplifier::mcp::Mcp;
use askama::Template;
use axum::response::sse::KeepAlive;
use mcp230xx::Mcp23017;
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
use futures_util::stream::Stream;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::io::Error;
use std::path;
use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex, mpsc};
use std::thread;
use std::{
    convert::Infallible,
    path::PathBuf,
    time::{Duration, Instant},
};
use tokio::sync::broadcast::{self, Sender};
use tokio::process::Command;
use tokio::time::{interval, sleep, timeout};
use tokio_tungstenite::connect_async;
use tokio_tungstenite::tungstenite::Message;
use tower_http::{services::ServeDir, trace::TraceLayer};
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};
const ENABLE_PIN: u8 = 16;
const DEFAULT_WATCHDOG_SECS: u64 = 15;
const TUNE_HOME_TOLERANCE_STEPS: i32 = 20;
const ALL_BAND_KEYS: [&str; 6] = ["10M", "11M", "15M", "20M", "40M", "80M"];

fn default_watchdog_secs() -> u64 {
    DEFAULT_WATCHDOG_SECS
}

fn first_available_profile_name() -> Option<String> {
    let static_dir = env::current_dir().ok()?.join("static");
    let mut files: Vec<String> = fs::read_dir(static_dir)
        .ok()?
        .filter_map(Result::ok)
        .filter_map(|entry| {
            let name = entry.file_name().to_string_lossy().to_string();
            name.ends_with(".json").then_some(name)
        })
        .collect();
    files.sort_unstable();
    files.into_iter().next()
}

#[derive(Template)]
#[template(path = "amplifier2.html")]
struct IndexTemplate {}
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
    call_sign: String,
    tci_server: String,
    follow_me: bool,
    tci_status: String,
    tci_watchdog_secs: u64,
    default_profile: String,
    cat_enabled: bool,
    cat_status: String,
    cat_watchdog_secs: u64,
    rig_model_id: i32,
    rig_serial_device: String,
    rig_baud: u32,
    rig_civaddr: String,
    rig_extra_conf: String,
    tune_reference_pin: String,
    tune_reference_active_low: bool,
    tune_homed: bool,
    tune_fault: String,
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
    #[serde(default = "default_watchdog_secs")]
    tci_watchdog_secs: u64,
    #[serde(default)]
    cat_enabled: bool,
    #[serde(default)]
    cat_status: String,
    #[serde(default = "default_watchdog_secs")]
    cat_watchdog_secs: u64,
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
    #[serde(default)]
    tune_reference_pin: Option<u8>,
    #[serde(default = "default_true")]
    tune_reference_active_low: bool,
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
            tci_watchdog_secs: DEFAULT_WATCHDOG_SECS,
            cat_enabled: false,
            cat_status: String::new(),
            cat_watchdog_secs: DEFAULT_WATCHDOG_SECS,
            rigctld_host: "127.0.0.1".to_string(),
            rigctld_port: 4532,
            rig_model_id: 0,
            rig_serial_device: String::new(),
            rig_baud: 0,
            rig_civaddr: String::new(),
            rig_extra_conf: String::new(),
            tune_reference_pin: None,
            tune_reference_active_low: true,
        }
    }
}
fn default_true() -> bool { true }
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
    pending_tci_band: Option<Bands>,
    tci_status: String,
    tci_watchdog_secs: u64,
    cat_enabled: bool,
    cat_status: String,
    cat_watchdog_secs: u64,
    rigctld_host: String,
    rigctld_port: u16,
    rig_model_id: i32,
    rig_serial_device: String,
    rig_baud: u32,
    rig_civaddr: String,
    rig_extra_conf: String,
    last_cat_band: Option<Bands>,
    pending_cat_band: Option<Bands>,
    default_profile: String,
    meter_sender: Option<mpsc::Sender<bool>>,
    tune_reference_pin: Option<u8>,
    tune_reference_active_low: bool,
    tune_homed: bool,
    tune_fault: Option<String>,
}
#[derive(Clone, Copy, Serialize, Deserialize)]
enum Select {
    Tune,
    Ind,
    Load,
}
#[derive(Clone, Copy, Serialize, Deserialize, Debug, PartialEq, Eq)]
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
#[allow(non_snake_case)]
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
    fn new() -> Result<Self, String> {
        let mut mcp = Mcp::new()?;
        mcp.init()?;
        Ok(Self {
            Blwr: [*mcp.pins.get("A0").ok_or_else(|| "Missing MCP pin A0".to_string())?],
            Fil: [
                *mcp.pins.get("A1").ok_or_else(|| "Missing MCP pin A1".to_string())?,
                *mcp.pins.get("A2").ok_or_else(|| "Missing MCP pin A2".to_string())?,
            ],
            HV: [
                *mcp.pins.get("A3").ok_or_else(|| "Missing MCP pin A3".to_string())?,
                *mcp.pins.get("A4").ok_or_else(|| "Missing MCP pin A4".to_string())?,
            ],
            Oper: [*mcp.pins.get("A5").ok_or_else(|| "Missing MCP pin A5".to_string())?],
            bands: [
                *mcp.pins.get("B0").ok_or_else(|| "Missing MCP pin B0".to_string())?,
                *mcp.pins.get("B1").ok_or_else(|| "Missing MCP pin B1".to_string())?,
                *mcp.pins.get("B2").ok_or_else(|| "Missing MCP pin B2".to_string())?,
                *mcp.pins.get("B3").ok_or_else(|| "Missing MCP pin B3".to_string())?,
                *mcp.pins.get("B4").ok_or_else(|| "Missing MCP pin B4".to_string())?,
            ],
            mcp,
        })
    }
}

#[tokio::main]
async fn main() -> Result<(), std::io::Error> {
    let (tx, _rx) = broadcast::channel(1024);
    let enable_pin = {
        let gpio = Gpio::new()
            .map_err(|err| std::io::Error::other(format!("Enable GPIO init failed: {err}")))?;
        let mut pin = gpio
            .get(ENABLE_PIN)
            .map_err(|err| std::io::Error::other(format!("Enable pin {ENABLE_PIN} init failed: {err}")))?
            .into_output();
        pin.set_high();
        Arc::new(Mutex::new(pin))
    };
    let pwr_btns = PwrBtns::new()
        .map_err(std::io::Error::other)?;
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
        enable_pin,
        pwr_btns,
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
        pending_tci_band: None,
        tci_status: "DISCONNECTED".to_string(),
        tci_watchdog_secs: DEFAULT_WATCHDOG_SECS,
        cat_enabled: false,
        cat_status: "DISCONNECTED".to_string(),
        cat_watchdog_secs: DEFAULT_WATCHDOG_SECS,
        rigctld_host: "127.0.0.1".to_string(),
        rigctld_port: 4532,
        rig_model_id: 0,
        rig_serial_device: String::new(),
        rig_baud: 0,
        rig_civaddr: String::new(),
        rig_extra_conf: String::new(),
        last_cat_band: None,
        pending_cat_band: None,
        default_profile: String::new(),
        meter_sender: None,
        tune_reference_pin: None,
        tune_reference_active_low: true,
        tune_homed: false,
        tune_fault: None,
    }));
    {
        let (tx, _rx) = mpsc::channel();
        app_state.lock().unwrap().meter_sender = Some(tx);
    }
    if let Some(profile_name) = read_default_profile_name() {
        match load_profile_from_file(app_state.clone(), &profile_name) {
            Ok(()) => app_state.lock().unwrap().default_profile = profile_name,
            Err(err) => {
                app_state.lock().unwrap().status = format!("Default profile load failed: {err}");
            }
        }
    } else if let Some(profile_name) = first_available_profile_name() {
        match load_profile_from_file(app_state.clone(), &profile_name) {
            Ok(()) => {
                app_state.lock().unwrap().status =
                    format!("No default profile configured. Loaded {profile_name} as startup fallback.");
            }
            Err(err) => {
                app_state.lock().unwrap().status =
                    format!("No default profile configured and fallback load failed: {err}");
            }
        }
    } else {
        app_state.lock().unwrap().status =
            "No default profile configured and no saved profiles were found.".to_string();
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
    let bind_addr = env::var("AMPLIFIER_BIND").unwrap_or_else(|_| "0.0.0.0:3000".to_string());
    let listener = tokio::net::TcpListener::bind(&bind_addr)
        .await
        .map_err(|err| {
            eprintln!("Failed to bind amplifier HTTP listener on {bind_addr}: {err}");
            err
        })?;
    if let Ok(addr) = listener.local_addr() {
        tracing::debug!("listening on {}", addr);
    }
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
                let template = IndexTemplate {};
                Html(template.render().unwrap_or_else(|_| "Template render failed".to_string()))
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
    let form_data = match process_form(form).await {
        Ok(data) => data,
        Err(err) => {
            let mut state_lck = state.lock().unwrap();
            state_lck.status = format!("Invalid config form data: {}", err);
            return Redirect::to("/config");
        }
    };
    let mut state = state.lock().unwrap();
    let mut persist_config = false;
    println!("FormData: {:?}", form_data);
    if form_data.contains_key("call_sign") {
        state.call_sign = form_data.get("call_sign").unwrap().trim().to_string();
        println!("Callsign added: {}", state.call_sign);
        persist_config = true;
    }
    if form_data.contains_key("save_tune_reference")
        || form_data.contains_key("tune_reference_pin")
        || form_data.contains_key("tune_reference_active_low")
    {
        let new_pin = match form_data.get("tune_reference_pin").map(|pin| pin.trim()) {
            Some("") | Some("None") | None => None,
            Some(pin) => match pin.parse::<u8>() {
                Ok(pin) => Some(pin),
                Err(_) => {
                    state.status = "Invalid Tune reference GPIO".to_string();
                    return Redirect::to("/config");
                }
            },
        };
        let mut tune_reference_pin = state.tune_reference_pin;
        if let Err(err) = assign_optional_pin(&mut state.gpio_pins, &mut tune_reference_pin, new_pin) {
            state.status = err;
            return Redirect::to("/config");
        }
        state.tune_reference_pin = tune_reference_pin;
        state.tune_reference_active_low = form_data.contains_key("tune_reference_active_low");
        state.tune_homed = false;
        clear_tune_fault(&mut state);
        refresh_tune_reference_status(&mut state);
        state.status = match state.tune_reference_pin {
            Some(pin) => format!("Tune reference sensor configured on GPIO {pin}"),
            None => "Tune reference sensor cleared".to_string(),
        };
        persist_config = true;
    }
    if form_data.contains_key("save_tci")
        || form_data.contains_key("start_tci")
        || form_data.contains_key("stop_tci")
        || form_data.contains_key("tci_server")
        || form_data.contains_key("follow_me")
        || form_data.contains_key("tci_watchdog_secs")
    {
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
        if let Some(secs) = form_data.get("tci_watchdog_secs") {
            let secs = secs.trim();
            if secs.is_empty() {
                state.tci_watchdog_secs = DEFAULT_WATCHDOG_SECS;
            } else if let Ok(parsed) = secs.parse::<u64>() {
                state.tci_watchdog_secs = parsed.max(3);
            } else {
                state.status = "Invalid TCI watchdog seconds".to_string();
                return Redirect::to("/config");
            }
        }
        if form_data.contains_key("start_tci") {
            state.follow_me = true;
            state.cat_enabled = false;
            state.pending_cat_band = None;
            state.cat_status = "DISCONNECTED".to_string();
        } else if form_data.contains_key("stop_tci") {
            state.follow_me = false;
        } else if let Some(follow) = form_data.get("follow_me") {
            state.follow_me = follow == "on";
        }
        if !state.follow_me {
            state.pending_tci_band = None;
        }
        state.status = format!(
            "TCI settings updated (Follow Me: {}, watchdog: {}s)",
            if state.follow_me { "ON" } else { "OFF" },
            state.tci_watchdog_secs
        );
        persist_config = true;
    }
    if form_data.contains_key("save_cat")
        || form_data.contains_key("start_cat")
        || form_data.contains_key("stop_cat")
        || form_data.contains_key("cat_enabled")
        || form_data.contains_key("cat_watchdog_secs")
        || form_data.contains_key("rigctld_host")
        || form_data.contains_key("rigctld_port")
        || form_data.contains_key("rig_model_id")
        || form_data.contains_key("rig_serial_device")
        || form_data.contains_key("rig_baud")
        || form_data.contains_key("rig_civaddr")
        || form_data.contains_key("rig_extra_conf")
    {
        if form_data.contains_key("start_cat") {
            state.cat_enabled = true;
            state.follow_me = false;
            state.pending_tci_band = None;
            state.tci_status = "PAUSED".to_string();
        } else if form_data.contains_key("stop_cat") {
            state.cat_enabled = false;
        } else {
            state.cat_enabled = form_data.get("cat_enabled").map(|v| v == "on").unwrap_or(false);
        }
        if !state.cat_enabled {
            state.pending_cat_band = None;
        }
        if let Some(secs) = form_data.get("cat_watchdog_secs") {
            let secs = secs.trim();
            if secs.is_empty() {
                state.cat_watchdog_secs = DEFAULT_WATCHDOG_SECS;
            } else if let Ok(parsed) = secs.parse::<u64>() {
                state.cat_watchdog_secs = parsed.max(3);
            } else {
                state.status = "Invalid CAT watchdog seconds".to_string();
                return Redirect::to("/config");
            }
        }
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
        if form_data.contains_key("rig_model_id") {
            match parse_optional_i32_field(&form_data, "rig_model_id", "rig model ID") {
                Ok(Some(model_id)) => state.rig_model_id = model_id,
                Ok(None) => state.rig_model_id = 0,
                Err(err) => {
                    state.status = err;
                    return Redirect::to("/config");
                }
            }
        }
        if let Some(dev) = form_data.get("rig_serial_device") {
            state.rig_serial_device = dev.trim().to_string();
        }
        if form_data.contains_key("rig_baud") {
            match parse_optional_u32_field(&form_data, "rig_baud", "rig baud") {
                Ok(Some(baud)) => state.rig_baud = baud,
                Ok(None) => state.rig_baud = 0,
                Err(err) => {
                    state.status = err;
                    return Redirect::to("/config");
                }
            }
        }
        if let Some(addr) = form_data.get("rig_civaddr") {
            state.rig_civaddr = addr.trim().to_string();
        }
        if let Some(extra) = form_data.get("rig_extra_conf") {
            state.rig_extra_conf = extra.trim().to_string();
        }
        state.status = format!(
            "CAT settings updated (Auto band: {}, watchdog: {}s)",
            if state.cat_enabled { "ON" } else { "OFF" },
            state.cat_watchdog_secs
        );
        persist_config = true;
    }
    if state.cat_enabled && state.follow_me {
        state.follow_me = false;
        state.pending_tci_band = None;
        state.status = "CAT and TCI cannot both be enabled; CAT kept ON, TCI turned OFF".to_string();
        persist_config = true;
    }
    if persist_config {
        if let Err(err) = persist_current_profile(&mut state, false) {
            state.status = format!("Settings updated but profile save failed: {}", err);
        }
    }

    if state.enc.is_some()  {
        if form_data.contains_key("del_enc") {
            let pin_a = state.enc.clone().unwrap().pin_a;
            let pin_b = state.enc.clone().unwrap().pin_b;
            let _ = process_pins(&mut state.gpio_pins, pin_a, false);
            let _ = process_pins(&mut state.gpio_pins, pin_b, false);
            state.enc.clone().unwrap().stop();
            state.enc = None;
            state.status = "Encoder has benn deleted!".to_string();
            
        }
        else if form_data.contains_key("add_tune") {
            if state.tune.lock().unwrap().pin_a.is_some() {
                println!("PinA already initialized for Tune");
            } else {
                handle_stepper(&mut state, form_data.clone(),  "Tune", true,|state| state.tune.clone());
                
            }
        }
        else if form_data.contains_key("del_tune") {
            handle_stepper(&mut state, form_data.clone(),  "Tune", false, |state| state.tune.clone()); 
        }
        else if form_data.contains_key("add_ind") {
            if state.ind.lock().unwrap().pin_a.is_some() {
                println!("PinA already initialized for Ind");
            } else {
                handle_stepper(&mut state, form_data.clone(),  "Ind", true,|state| state.ind.clone()); 
            }
        }
        else if form_data.contains_key("del_ind") {
            handle_stepper(&mut state, form_data.clone(),  "Ind", false ,|state| state.ind.clone()); 
        }
        else if form_data.contains_key("add_load") {
            if state.load.lock().unwrap().pin_a.is_some() {
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
            let Some(start_target) = form_data.get("start").map(String::as_str) else {
                state.status = "Missing start target".to_string();
                return Redirect::to("/config");
            };
            match start_target {
                "tune" => {
                    if let Some(tx) = state.meter_sender.clone() {
                        let _ = tx.send(false);
                    }
                    let state_tune = state.tune.lock().unwrap();
                    state_tune.pos.store(0, Ordering::Relaxed);
                    drop(state_tune);
                    state.tune_homed = true;
                    clear_tune_fault(&mut state);
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
            let Some(max_target) = form_data.get("max").map(String::as_str) else {
                state.status = "Missing max target".to_string();
                return Redirect::to("/config");
            };
            match max_target {
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
            let Some(reset_target) = form_data.get("reset").map(String::as_str) else {
                state.status = "Missing reset target".to_string();
                return Redirect::to("/config");
            };
            match reset_target {
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
    } else if form_data.contains_key("PinA") && form_data.contains_key("PinB") {
        let pin_a = match parse_optional_u8_field(&form_data, "PinA", "encoder Pin A") {
            Ok(Some(pin)) => pin,
            Ok(None) => return Redirect::to("/config"),
            Err(err) => {
                state.status = err;
                return Redirect::to("/config");
            }
        };
        let pin_b = match parse_optional_u8_field(&form_data, "PinB", "encoder Pin B") {
            Ok(Some(pin)) => pin,
            Ok(None) => return Redirect::to("/config"),
            Err(err) => {
                state.status = err;
                return Redirect::to("/config");
            }
        };
        state.enc = Some(Encoder::new(
            pin_a,
            pin_b,
        ));
        if let Err(err) = state.enc.clone().unwrap().run() {
            state.status = err;
            state.enc = None;
            return Redirect::to("/config");
        }
        let _ = process_pins(&mut state.gpio_pins, pin_a, true);
        let _ = process_pins(&mut state.gpio_pins, pin_b, true);
        println!("Encoder Added");
        state.status = format!(
            "Encoder Added on pins: {:?}, {:?}",
            pin_a,
            pin_b,
        );
    }
    Redirect::to("/config")
}

fn process_pins(pin_list: &mut Vec<u8>, val: u8, remove: bool) -> Result<(), Box< dyn std::error::Error>> {
    if remove {
        if let Some(out) = pin_list.iter().position(|&x| x == val) {
            pin_list.remove(out);
            Ok(())
        } else {
            Err(Box::new(Error::other("Pin not Found")))
        }
    } else {
        pin_list.push(val);
        Ok(())
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
        enc: state.enc.is_some(),
        enc_val: if let Some(enc) = state.enc.clone() {
            vec![enc.pin_a.to_string(), enc.pin_b.to_string()]
        } else {
            vec!["None".to_string(), "None".to_string()]
        },
        tune: if let (Some(pin_a), Some(pin_b)) = (tune.pin_a, tune.pin_b) {
            vec![pin_a.to_string(), pin_b.to_string(), tune.ratio.to_string()]
        } else {
            vec!["None".to_string(), "None".to_string(), 1.to_string()]
        },
        ind: if let (Some(pin_a), Some(pin_b)) = (ind.pin_a, ind.pin_b) {
            vec![pin_a.to_string(), pin_b.to_string(), ind.ratio.to_string()]
        } else {
            vec!["None".to_string(), "None".to_string(), 1.to_string()]
        },
        load: if let (Some(pin_a), Some(pin_b)) = (load.pin_a, load.pin_b) {
            vec![pin_a.to_string(), pin_b.to_string(), load.ratio.to_string()]
        } else {
            vec!["None".to_string(), "None".to_string(), 1.to_string()]
        },
        files: {
            let mut files = Vec::new();
            if let Ok(home_path) = env::current_dir().map(|dir| dir.join("static"))
                && let Ok(entries) = fs::read_dir(home_path)
            {
                files = entries
                    .filter_map(Result::ok)
                    .map(|entry| entry.file_name().to_string_lossy().to_string())
                    .filter(|name| name.ends_with(".json"))
                    .collect();
                files.sort_unstable();
            }
            files
        },
        pins: state.gpio_pins.clone(),
        call_sign: state.call_sign.clone(),
        tci_server: state.tci_server.clone(),
        follow_me: state.follow_me,
        tci_status: state.tci_status.clone(),
        tci_watchdog_secs: state.tci_watchdog_secs,
        default_profile: state.default_profile.clone(),
        cat_enabled: state.cat_enabled,
        cat_status: state.cat_status.clone(),
        cat_watchdog_secs: state.cat_watchdog_secs,
        rig_model_id: state.rig_model_id,
        rig_serial_device: state.rig_serial_device.clone(),
        rig_baud: state.rig_baud,
        rig_civaddr: state.rig_civaddr.clone(),
        rig_extra_conf: state.rig_extra_conf.clone(),
        tune_reference_pin: state
            .tune_reference_pin
            .map(|pin| pin.to_string())
            .unwrap_or_else(|| "None".to_string()),
        tune_reference_active_low: state.tune_reference_active_low,
        tune_homed: state.tune_homed,
        tune_fault: state.tune_fault.clone().unwrap_or_default(),
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
    s.split([';', '\n', '\r'])
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

fn set_tune_fault(state: &mut AppState, message: impl Into<String>) {
    let message = message.into();
    state.tune_fault = Some(message.clone());
    state.tune_homed = false;
    state.status = format!("Tune fault: {message}");
}

fn clear_tune_fault(state: &mut AppState) {
    state.tune_fault = None;
}

fn read_reference_sensor(pin: u8, active_low: bool) -> Result<bool, String> {
    let gpio = Gpio::new().map_err(|e| format!("Reference GPIO init failed: {e}"))?;
    let level = gpio
        .get(pin)
        .map_err(|e| format!("Reference pin {pin} init failed: {e}"))?
        .into_input_pullup()
        .read();
    Ok(if active_low {
        level == rppal::gpio::Level::Low
    } else {
        level == rppal::gpio::Level::High
    })
}

fn refresh_tune_reference_status(state: &mut AppState) {
    let Some(pin) = state.tune_reference_pin else {
        state.tune_homed = false;
        state.tune_fault = None;
        return;
    };
    let tune_pos = state.tune.lock().unwrap().pos.load(Ordering::Relaxed);
    match read_reference_sensor(pin, state.tune_reference_active_low) {
        Ok(true) => {
            if tune_pos.abs() <= TUNE_HOME_TOLERANCE_STEPS {
                state.tune_homed = true;
                clear_tune_fault(state);
            } else {
                set_tune_fault(
                    state,
                    format!(
                        "reference sensor active on GPIO {pin}, but tune position is {tune_pos}"
                    ),
                );
            }
        }
        Ok(false) => {
            if state.tune_homed && tune_pos == 0 {
                state.tune_homed = false;
            }
        }
        Err(err) => set_tune_fault(state, err),
    }
}

fn assign_optional_pin(
    gpio_pins: &mut Vec<u8>,
    current: &mut Option<u8>,
    new_pin: Option<u8>,
) -> Result<(), String> {
    if *current == new_pin {
        return Ok(());
    }
    if let Some(existing) = *current {
        if !gpio_pins.contains(&existing) {
            gpio_pins.push(existing);
            gpio_pins.sort_unstable();
        }
    }
    if let Some(pin) = new_pin {
        if let Some(index) = gpio_pins.iter().position(|&available| available == pin) {
            gpio_pins.remove(index);
        } else {
            return Err(format!("GPIO {pin} is already reserved"));
        }
    }
    *current = new_pin;
    Ok(())
}

fn wait_for_stepper_shutdown(stepper: &Arc<Mutex<Stepper>>) {
    for _ in 0..50 {
        if !stepper.lock().unwrap().is_running() {
            break;
        }
        thread::sleep(Duration::from_millis(10));
    }
}

fn wait_for_encoder_shutdown(enc: &Encoder) {
    for _ in 0..50 {
        if !enc.is_running() {
            break;
        }
        thread::sleep(Duration::from_millis(10));
    }
}

fn validate_stepper_profile(stepper_name: &str, data: &HashMap<String, u32>) -> Result<(), String> {
    for key in ["pos", "max", "ratio"] {
        if !data.contains_key(key) {
            return Err(format!("Profile missing {stepper_name}.{key}"));
        }
    }
    Ok(())
}

fn parse_required_u8_field(
    form_data: &HashMap<String, String>,
    key: &str,
    label: &str,
) -> Result<u8, String> {
    let raw = form_data
        .get(key)
        .map(|value| value.trim())
        .ok_or_else(|| format!("Missing {label}"))?;
    if raw.is_empty() {
        return Err(format!("{label} is required"));
    }
    raw.parse::<u8>()
        .map_err(|_| format!("Invalid {label}: {raw}"))
}

fn parse_optional_u8_field(
    form_data: &HashMap<String, String>,
    key: &str,
    label: &str,
) -> Result<Option<u8>, String> {
    match form_data.get(key).map(|value| value.trim()) {
        Some("") | None => Ok(None),
        Some(raw) => raw
            .parse::<u8>()
            .map(Some)
            .map_err(|_| format!("Invalid {label}: {raw}")),
    }
}

fn parse_optional_i32_field(
    form_data: &HashMap<String, String>,
    key: &str,
    label: &str,
) -> Result<Option<i32>, String> {
    match form_data.get(key).map(|value| value.trim()) {
        Some("") | None => Ok(None),
        Some(raw) => raw
            .parse::<i32>()
            .map(Some)
            .map_err(|_| format!("Invalid {label}: {raw}")),
    }
}

fn parse_optional_u32_field(
    form_data: &HashMap<String, String>,
    key: &str,
    label: &str,
) -> Result<Option<u32>, String> {
    match form_data.get(key).map(|value| value.trim()) {
        Some("") | None => Ok(None),
        Some(raw) => raw
            .parse::<u32>()
            .map(Some)
            .map_err(|_| format!("Invalid {label}: {raw}")),
    }
}

fn try_recall_pending_band(
    state: Arc<Mutex<AppState>>,
    source: &str,
    pending_band: Option<Bands>,
) -> Option<(Bands, &'static str)> {
    let band = pending_band?;
    let mut state_lck = state.lock().unwrap();
    let source_enabled = match source {
        "Follow Me" => state_lck.follow_me && !state_lck.tci_server.is_empty() && !state_lck.cat_enabled,
        "CAT" => state_lck.cat_enabled,
        _ => true,
    };
    if !source_enabled {
        match source {
            "Follow Me" => state_lck.pending_tci_band = None,
            "CAT" => state_lck.pending_cat_band = None,
            _ => {}
        }
        return None;
    }
    let tune_busy = *state_lck.tune.lock().unwrap().operate.lock().unwrap();
    let ind_busy = *state_lck.ind.lock().unwrap().operate.lock().unwrap();
    let load_busy = *state_lck.load.lock().unwrap().operate.lock().unwrap();
    if tune_busy || ind_busy || load_busy || state_lck.band == band {
        return None;
    }
    let band_key = band_to_key(&band);
    state_lck.status = format!("{}: applying queued {}", source, band_key);
    Some((band, band_key))
}

async fn tci_follow_task(state: Arc<Mutex<AppState>>) {
    loop {
        let (server, enabled, cat_enabled, watchdog_secs) = {
            let state_lck = state.lock().unwrap();
            (
                state_lck.tci_server.clone(),
                state_lck.follow_me,
                state_lck.cat_enabled,
                state_lck.tci_watchdog_secs.max(3),
            )
        };
        let watchdog_timeout = Duration::from_secs(watchdog_secs);

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

        {
            let mut state_lck = state.lock().unwrap();
            state_lck.tci_status = "CONNECTING".to_string();
            state_lck.status = format!("TCI connecting: {}", server);
        }
        println!("TCI: connecting to {}", server);
        match connect_async(server.as_str()).await {
            Ok((mut ws, _)) => {
                let mut last_valid_tci_frame = Instant::now();
                {
                    let mut state_lck = state.lock().unwrap();
                    state_lck.tci_status = "CONNECTED".to_string();
                    state_lck.status = format!("TCI connected: {}", server);
                }
                println!("TCI: connected to {}", server);
                loop {
                    let pending_tci = {
                        let state_lck = state.lock().unwrap();
                        state_lck.pending_tci_band.clone()
                    };
                    if let Some((band_enum, band_key)) =
                        try_recall_pending_band(state.clone(), "Follow Me", pending_tci)
                    {
                        {
                            let mut state_lck = state.lock().unwrap();
                            state_lck.pending_tci_band = None;
                            state_lck.last_tci_band = Some(band_enum.clone());
                        }
                        match recall_handler(state.clone(), band_key.to_string(), band_enum, true) {
                            Ok(_) => println!("TCI: recall queued {}", band_key),
                            Err(e) => {
                                let mut state_lck = state.lock().unwrap();
                                state_lck.status = format!("TCI recall {} failed: {}", band_key, e);
                            }
                        }
                    }
                    tokio::select! {
                        msg = futures_util::StreamExt::next(&mut ws) => {
                            match msg {
                                Some(Ok(Message::Text(s))) => {
                                    for frame in split_frames(&s) {
                                        if let Some(hz) = parse_any_tx_hz(frame) {
                                            last_valid_tci_frame = Instant::now();
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
                                                    if !state_lck.follow_me
                                                        || state_lck.tci_server != server
                                                    {
                                                        None
                                                    } else {
                                                        let tune_busy = *state_lck.tune.lock().unwrap().operate.lock().unwrap();
                                                        let ind_busy = *state_lck.ind.lock().unwrap().operate.lock().unwrap();
                                                        let load_busy = *state_lck.load.lock().unwrap().operate.lock().unwrap();
                                                        if tune_busy || ind_busy || load_busy {
                                                            state_lck.pending_tci_band = Some(band.clone());
                                                            state_lck.status = format!(
                                                                "Follow Me: queued {} until tune completes",
                                                                band_to_key(&band)
                                                            );
                                                            None
                                                        } else if state_lck.band == band
                                                            || state_lck.last_tci_band == Some(band.clone())
                                                        {
                                                            state_lck.pending_tci_band = None;
                                                            state_lck.last_tci_band = Some(band.clone());
                                                            None
                                                        } else {
                                                            state_lck.pending_tci_band = None;
                                                            state_lck.last_tci_band = Some(band.clone());
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
                            let mut state_lck = state.lock().unwrap();
                            if !state_lck.follow_me || state_lck.tci_server != server {
                                break;
                            }
                            if last_valid_tci_frame.elapsed() > watchdog_timeout {
                                state_lck.tci_status = "ERROR".to_string();
                                state_lck.status = format!(
                                    "TCI watchdog: no frequency updates received for {}s, reconnecting",
                                    watchdog_secs
                                );
                                println!("TCI watchdog: stale connection, reconnecting");
                                break;
                            }
                        }
                    }
                }
                let mut state_lck = state.lock().unwrap();
                state_lck.tci_status = "DISCONNECTED".to_string();
                state_lck.status = format!("TCI disconnected: {}", server);
            }
            Err(_) => {
                {
                    let mut state_lck = state.lock().unwrap();
                    state_lck.status = format!("TCI connect failed: {}", server);
                    state_lck.tci_status = "ERROR".to_string();
                }
                println!("TCI: connect failed to {}", server);
                sleep(Duration::from_secs(2)).await;
            }
        }
    }
}

async fn cat_follow_task(state: Arc<Mutex<AppState>>) {
    let mut cat_connected = false;
    let mut last_valid_cat_poll = Instant::now();
    loop {
        let pending_cat = {
            let state_lck = state.lock().unwrap();
            state_lck.pending_cat_band.clone()
        };
        if let Some((band_enum, band_key)) =
            try_recall_pending_band(state.clone(), "CAT", pending_cat)
        {
            {
                let mut state_lck = state.lock().unwrap();
                state_lck.pending_cat_band = None;
                state_lck.last_cat_band = Some(band_enum.clone());
            }
            if let Err(e) = recall_handler(state.clone(), band_key.to_string(), band_enum, true) {
                let mut state_lck = state.lock().unwrap();
                state_lck.status = format!("CAT recall {} failed: {}", band_key, e);
            }
        }

        let (enabled, model_id, device, baud, civaddr, extra_conf, watchdog_secs) = {
            let state_lck = state.lock().unwrap();
            (
                state_lck.cat_enabled,
                state_lck.rig_model_id,
                state_lck.rig_serial_device.clone(),
                state_lck.rig_baud,
                state_lck.rig_civaddr.clone(),
                state_lck.rig_extra_conf.clone(),
                state_lck.cat_watchdog_secs.max(3),
            )
        };
        let watchdog_timeout = Duration::from_secs(watchdog_secs);

        if !enabled {
            {
                let mut state_lck = state.lock().unwrap();
                state_lck.cat_status = "DISCONNECTED".to_string();
            }
            cat_connected = false;
            last_valid_cat_poll = Instant::now();
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
            last_valid_cat_poll = Instant::now();
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
                    last_valid_cat_poll = Instant::now();
                    if let Some(band) = band_from_hz(hz) {
                        let maybe_recall = {
                            let mut state_lck = state.lock().unwrap();
                            if !cat_connected {
                                state_lck.cat_status = "CONNECTED".to_string();
                            }
                            if !state_lck.cat_enabled
                            {
                                None
                            } else {
                                let tune_busy = *state_lck.tune.lock().unwrap().operate.lock().unwrap();
                                let ind_busy = *state_lck.ind.lock().unwrap().operate.lock().unwrap();
                                let load_busy = *state_lck.load.lock().unwrap().operate.lock().unwrap();
                                if tune_busy || ind_busy || load_busy {
                                    state_lck.pending_cat_band = Some(band.clone());
                                    state_lck.status = format!(
                                        "CAT: queued {} until tune completes",
                                        band_to_key(&band)
                                    );
                                    None
                                } else if state_lck.band == band
                                    || state_lck.last_cat_band == Some(band.clone())
                                {
                                    state_lck.pending_cat_band = None;
                                    state_lck.last_cat_band = Some(band.clone());
                                    None
                                } else {
                                    state_lck.pending_cat_band = None;
                                    state_lck.last_cat_band = Some(band.clone());
                                    Some((band.clone(), band_to_key(&band)))
                                }
                            }
                        };
                        if let Some((band_enum, band_key)) = maybe_recall
                            && let Err(e) = recall_handler(state.clone(), band_key.to_string(), band_enum, true) {
                                let mut state_lck = state.lock().unwrap();
                                state_lck.status = format!("CAT recall {} failed: {}", band_key, e);
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
        if cat_connected && last_valid_cat_poll.elapsed() > watchdog_timeout {
            let mut state_lck = state.lock().unwrap();
            state_lck.cat_status = "ERROR".to_string();
            state_lck.status = format!(
                "CAT watchdog: no valid frequency updates received for {}s",
                watchdog_secs
            );
            cat_connected = false;
            println!("CAT watchdog: stale polling state");
        }
        sleep(Duration::from_millis(400)).await;
    }
}

//Selects a stepper to be tuned.
async fn selector(
    Path(val): Path<String>, State(app_state): State<Arc<Mutex<AppState>>>,
    form_data: Multipart,
) -> impl IntoResponse {
    println!("Form handler");
    println!("{}", val);
    app_state.lock().unwrap().enable_pin.lock().unwrap().set_low();
    let state_lck = app_state.lock().unwrap().clone();
    let tune = state_lck.tune.lock().unwrap().clone();
    let ind = state_lck.ind.lock().unwrap().clone();
    let load = state_lck.load.lock().unwrap().clone();
    if  !*tune.operate.lock().unwrap() && !*ind.operate.lock().unwrap() && !*load.operate.lock().unwrap() {
        let form_data = match process_form(form_data).await {
            Ok(data) => data,
            Err(err) => {
                let mut state = app_state.lock().unwrap();
                state.status = format!("Invalid selector form data: {}", err);
                return StatusCode::BAD_REQUEST;
            }
        };
        for key in form_data.keys() {
            println!("Name: {}", key);
            match key.as_str() {
                "tune" => {
                    let mut state = app_state.lock().unwrap();
                    if selector_handler(&mut state, |x| x.tune.clone()).is_ok() {
                        state.status = "Tune is selected".to_string();
                        state.sw_pos = Some(Select::Tune);
                    }
                }
                "ind" => {
                    let mut state = app_state.lock().unwrap();
                    if selector_handler(&mut state, |x| x.ind.clone()).is_ok() {
                        state.status = "Ind is selected".to_string();
                        state.sw_pos = Some(Select::Ind);
                        
                        
                    }
                }
                "load" => {
                    let mut state = app_state.lock().unwrap();
                    if selector_handler(&mut state, |x| x.load.clone()).is_ok() {
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
        app_state.lock().unwrap().status = "Cannot select a tuner while tune is in progress ! ! !".to_string();
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
        Ok(())
    } else {
        state.status = "No Encoder present! ! !".to_string();
        Err(Box::new(Error::other("No Encoder Forund")))
        
    }

}
//Recalls bands from memory.
async fn recall(Path(path): Path<String>, State(state): State<Arc<Mutex<AppState>>>) {
    println!("{}", path);
    let state_lck = state.lock().unwrap().clone();
        if !*state_lck.tune.lock().unwrap().operate.lock().unwrap() && !*state_lck.ind.lock().unwrap().operate.lock().unwrap() && !*state_lck.load.lock().unwrap().operate.lock().unwrap()  {
            state.lock().unwrap().sleep = true;
            match path.as_str() {
                "M10" => {
                    let _ = recall_handler(state.clone(), "10M".to_string(), Bands::M10, false);
                }
                "M11" => {
                    let _ = recall_handler(state.clone(), "11M".to_string(), Bands::M11, false);
                }
                "M15" => {
                    let _ = recall_handler(state.clone(), "15M".to_string(), Bands::M15, false);
                }
                "M20" => {
                    let _ = recall_handler(state.clone(), "20M".to_string(), Bands::M20, false);
                }
                "M40" => {
                    let _ = recall_handler(state.clone(), "40M".to_string(), Bands::M40, false);
                }
                "M80" => {
                    let _ = recall_handler(state.clone(), "80M".to_string(), Bands::M80, false);
                }
                _ => {
                    println!("Invalid band selected!!")
                }
            }
        } else {
        state.lock().unwrap().status = "Attempted to recall while motors still in motion!!".to_string();
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
    let form_data = match process_form(form).await {
        Ok(data) => data,
        Err(err) => {
            let mut state_lck = state.lock().unwrap();
            state_lck.status = format!("Invalid load form data: {}", err);
            return Redirect::to("/config");
        }
    };
    if form_data.contains_key("clear_default") {
        let _ = clear_default_profile_name();
        let mut state_lck = state.lock().unwrap();
        state_lck.default_profile = String::new();
        state_lck.status = "Default profile cleared".to_string();
    } else if form_data.contains_key("files") && form_data.contains_key("load") {
        let file_name = form_data.get("files").unwrap();
        println!("Filename: {}", file_name);
        match load_profile_from_file(state.clone(), file_name) {
            Ok(()) => {
                let mut status = format!("Loaded profile: {}", file_name);
                let mut state_lck = state.lock().unwrap();
                if form_data.contains_key("default_profile") {
                    if let Err(err) = write_default_profile_name(file_name) {
                        status = format!("Loaded profile but failed to set default: {}", err);
                    } else {
                        state_lck.default_profile = file_name.to_string();
                        status = format!("Loaded profile and set default: {}", file_name);
                    }
                }
                state_lck.status = status;
            }
            Err(err) => {
                let mut state_lck = state.lock().unwrap();
                state_lck.status = format!("Failed to load profile {}: {}", file_name, err);
            }
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
    Redirect::to("/config")
}

//power button handler.
async fn pwr_btn_handler(State(state): State<Arc<Mutex<AppState>>>, form: Multipart) {
    let form_data = match process_form(form).await {
        Ok(data) => data,
        Err(err) => {
            let mut state_lck = state.lock().unwrap();
            state_lck.status = format!("Invalid power control form data: {}", err);
            return;
        }
    };
    if form_data.contains_key("ID") {
        let sw = form_data.get("ID").unwrap();
        println!("Switch: {}", sw);
        let Some(action) = form_data.get("value") else {
            state.lock().unwrap().status = "Missing power control action".to_string();
            return;
        };
        println!("Action: {}", action);
        {
            let mut state_lck = state.lock().unwrap();
            refresh_tune_reference_status(&mut state_lck);
            if action == "ON"
                && matches!(sw.as_str(), "HV" | "Oper")
                && (state_lck.tune_fault.is_some()
                    || (state_lck.tune_reference_pin.is_some() && !state_lck.tune_homed))
            {
                state_lck.status =
                    "Blocked power-on: Tune is not homed or has a reference fault".to_string();
                return;
            }
        }
        match sw.as_str() {
            "Blwr" => {
                let mut state_lck = state.lock().unwrap();
                let pin = state_lck.pwr_btns.Blwr[0];
                state_lck.pwr_btns.mcp.set_pin(pin, if action == "ON" {mcp230xx::Level::High} else {mcp230xx::Level::Low}).unwrap_or(());
                state_lck.status = (if action == "ON" {"Blower ON"} else {"Blower OFF"}).to_string();

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
                state_lck.status = (if action == "ON" {"Operate"} else {"Standby"}).to_string();

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
        let Some(action) = form_data.get("value") else {
            state_lck.status = format!("Missing {} power action", name);
            return;
        };
        let my_btns = callback(state_lck);
        let pin1 = my_btns[0];
        let pin2 = my_btns[1];
        let pin1_status = match state_lck.pwr_btns.mcp.read_pin(pin1) {
            Ok(level) => level,
            Err(err) => {
                state_lck.status = format!("Power sequencing read failed: {err}");
                return;
            }
        };
        let _ = state_lck.pwr_btns.mcp.set_pin(pin1, if action == "ON" {mcp230xx::Level::High} else {mcp230xx::Level::Low});  
        if form_data.contains_key("delay") {
            let delay = form_data.get("delay").unwrap();
            let _ = state_lck.pwr_btns.mcp.set_pin(pin2, if delay == "ON"  && pin1_status == mcp230xx::Level::High {mcp230xx::Level::High} else {mcp230xx::Level::Low});
            state_lck.status = (if action == "ON" && delay == "OFF" {
                format!("{} Step Start !!!",  name)
            } else if pin1_status == mcp230xx::Level::High && delay == "ON" {
                format!("{}  ON ! ! !", name)
            } else {
                format!("{} Shutting Down...", name)
            }).to_string();
        } 
    }

fn normalized_stepper_max(stepper: &Stepper) -> i32 {
    let mut max_value = stepper.max.load(Ordering::Relaxed).max(stepper.pos.load(Ordering::Relaxed));
    for value in stepper.mem.values() {
        max_value = max_value.max(value.load(Ordering::Relaxed));
    }
    max_value.max(0)
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
        if !*tune.operate.lock().unwrap() && !*ind.operate.lock().unwrap() && !*load.operate.lock().unwrap() && val.sleep {
            count += 1;
            if count >= 10 {
                sleep_save(state.clone());
                count = 0;
            }
        } else {
            count = 0;
        }
        if val.enc.is_some() {
            let clone = val.enc.clone().unwrap().enc();
            if clone >= 0 {
                match val.sw_pos {
                    Some(Select::Tune) => {
                        let tune_max = normalized_stepper_max(&tune);
                        if clone <= tune_max && clone >= 0 {
                            if tune.pin_a.is_some() {
                                if let Some(ch) = tune.channel.clone() {
                                    let _ = ch.send((clone as u32, false));
                                }
                            } else {
                                tune.pos.store(clone, Ordering::Relaxed);
                            }
                        }
                    }
                    Some(Select::Ind) => {
                        let ind_max = normalized_stepper_max(&ind);
                        if clone <= ind_max && clone >= 0 {
                            if ind.pin_a.is_some() {
                                if let Some(ch) = ind.channel.clone() {
                                    let _ = ch.send((clone as u32, false));
                                }
                            } else {
                                ind.pos.store(clone, Ordering::Relaxed);
                            }
                        }
                    }
                    Some(Select::Load) => {
                        let load_max = normalized_stepper_max(&load);
                        if clone <= load_max && clone >= 0 {
                            if load.pin_a.is_some() {
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
        match serde_json::to_string(&sse_output) {
            Ok(payload) => {
                let _ = val.sender.send(payload);
            }
            Err(err) => {
                eprintln!("SSE serialization failed: {err}");
            }
        }
    }
}

//aquires I2C data and loads it to the AppState global Mutex.
async fn aquire_i2c_data(state: Arc<Mutex<AppState>>) {
    let mut interval = interval(Duration::from_millis(100));
    let (tx, rx) = mpsc::channel();
    state.lock().unwrap().meter_sender = Some(tx);
    let mut run = true;
    let mut pin_fault_active = false;
    let mut meter_fault_active = false;
    loop {
        interval.tick().await;
        let mut val = state.lock().unwrap().pwr_btns.clone();
        let mut temp_data: HashMap<String, [String; 2]> = HashMap::from([
            ("Blwr".to_string(), ["OFF".to_string(), "OFF".to_string()]),
            ("Fil".to_string(), ["OFF".to_string(), "OFF".to_string()]),
            ("HV".to_string(), ["OFF".to_string(), "OFF".to_string()]),
            ("Oper".to_string(), ["OFF".to_string(), "OFF".to_string()]),
        ]);

        let mut pin_read_failed = false;
        let mut read_level = |pin| match val.mcp.read_pin(pin) {
            Ok(level) => {
                if pin_fault_active {
                    println!("I2C: power button pin reads recovered");
                    pin_fault_active = false;
                }
                if level == mcp230xx::Level::High { "ON".to_string() } else { "OFF".to_string() }
            }
            Err(err) => {
                pin_read_failed = true;
                if !pin_fault_active {
                    eprintln!("I2C: failed to read power button pin state: {}", err);
                    pin_fault_active = true;
                }
                "OFF".to_string()
            }
        };

        temp_data.insert("Blwr".to_string(), [read_level(val.Blwr[0]), "OFF".to_string()]);
        temp_data.insert(
            "Fil".to_string(),
            [read_level(val.Fil[0]), read_level(val.Fil[1])],
        );
        temp_data.insert(
            "HV".to_string(),
            [read_level(val.HV[0]), read_level(val.HV[1])],
        );
        temp_data.insert("Oper".to_string(), [read_level(val.Oper[0]), "OFF".to_string()]);
        if let Ok(val) = rx.try_recv() {
            run = val;
        }
        let mut temp = 0.0;
        let mut screen_ma = 0_u32;
        let mut plate_v = 0_u32;
        let mut meter_read_failed = false;
        if run
            && let Ok(t)=  val.mcp.read_val() {
                plate_v = t.2 as u32;
                screen_ma = t.1 as u32;
                temp = t.0;
                if meter_fault_active {
                    println!("I2C: meter reads recovered");
                    meter_fault_active = false;
                }
            } else if run {
                meter_read_failed = true;
                if !meter_fault_active {
                    eprintln!("I2C: failed to read meter values from MCP");
                    meter_fault_active = true;
                }
            }
        let mut state_lck = state.lock().unwrap();
        state_lck.pwr_btns_state = temp_data.clone();
        state_lck.temperature = temp;
        state_lck.gauges.screen_a = screen_ma;
        state_lck.gauges.plate_v = plate_v * 100;
        let i2c_status = if pin_read_failed {
            Some("I2C warning: failed to read power button state")
        } else if meter_read_failed {
            Some("I2C warning: failed to read meter values")
        } else if pin_fault_active || meter_fault_active {
            Some("I2C warning: hardware read error")
        } else {
            None
        };
        match i2c_status {
            Some(message) => {
                if state_lck.status.is_empty() || state_lck.status.starts_with("I2C warning:") {
                    state_lck.status = message.to_string();
                }
            }
            None => {
                if state_lck.status.starts_with("I2C warning:") {
                    state_lck.status.clear();
                }
            }
        }
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
        let pin_a = match parse_required_u8_field(&form_data, "PinA", &format!("{name} Pin A")) {
            Ok(pin) => pin,
            Err(err) => {
                state.status = err;
                return;
            }
        };
        let pin_b = match parse_required_u8_field(&form_data, "PinB", &format!("{name} Pin B")) {
            Ok(pin) => pin,
            Err(err) => {
                state.status = err;
                return;
            }
        };
        let ratio = match parse_optional_u8_field(&form_data, "ratio", &format!("{name} ratio")) {
            Ok(Some(ratio)) => ratio,
            Ok(None) => 1,
            Err(err) => {
                state.status = err;
                return;
            }
        };
        println!("Adding Stepper");
        state_stepper.name = name.to_string().to_lowercase();
        state_stepper.pin_a = Some(pin_a);
        state_stepper.pin_b = Some(pin_b);
        state_stepper.ratio = ratio;
        let _ = process_pins(&mut state.gpio_pins, pin_a, true);
        let _ = process_pins(&mut state.gpio_pins, pin_b, true);
        if name == "Ind" {
            state_stepper.speed = Duration::from_micros(400);
        }
        if let Err(err) = state_stepper.run_2() {
            state.status = err;
        }
    } else {
        println!("Resetting {} to default settings", name
    );
        if state_stepper.pin_a.is_some() {
            println!("Deleting {}", state_stepper.name);
            let pin_a = state_stepper.pin_a.unwrap();
            let pin_b = state_stepper.pin_b.unwrap();
            let _ = process_pins(&mut state.gpio_pins, pin_a, false);
            let _ = process_pins(&mut state.gpio_pins, pin_b, false);
            state_stepper.stop();
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
    if !add {
        wait_for_stepper_shutdown(&stepper);
    }
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
    refresh_tune_reference_status(&mut state_lck);
    if let Some(fault) = state_lck.tune_fault.clone() {
        return Err(Box::new(Error::other(format!("Tune reference fault: {fault}"))));
    }
    if require_stored
        && !state_lck
            .mem_valid
            .get(&band)
            .copied()
            .unwrap_or(false)
        {
            state_lck.status = format!("No stored settings for {} band", band);
            if band_enum == Bands::M11 {
                state_lck.band = band_enum.clone();
                return Ok(());
            }
            return Err(Box::new(Error::other("Band not stored")));
        }
    if state_lck.enc.is_some() {
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
            // Hardware wiring maps the last two band outputs in reverse order.
            Bands::M40 => {let _ = state_lck.pwr_btns.clone().mcp.set_pin(state_lck.pwr_btns.clone().bands[4], mcp230xx::Level::High);},
            Bands::M80 => {let _ = state_lck.pwr_btns.clone().mcp.set_pin(state_lck.pwr_btns.clone().bands[3], mcp230xx::Level::High);},
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
                let state_for_thread = state.clone();
                thread::spawn(move || {
                    let temp_lck = x.lock().unwrap().clone();
                    let Some(target_mem) = temp_lck.mem.get(&value) else {
                        state_for_thread.lock().unwrap().status =
                            format!("Recall failed: {} missing memory for {}", temp_lck.name, value);
                        return;
                    };
                    let target_pos = target_mem.load(Ordering::Relaxed);
                    if temp_lck.pin_a.is_some() {
                        if let Some(channel) = temp_lck.channel.clone() {
                            let _ = channel.send((target_pos as u32, false));
                        } else {
                            state_for_thread.lock().unwrap().status =
                                format!("Recall failed: {} motor channel is unavailable", temp_lck.name);
                            return;
                        }
                    } else {
                        temp_lck.pos.store(target_pos, Ordering::Relaxed);
                    }
                    println!("Run thread ended");

                });
                
            }
            let mut state_lck = state.lock().unwrap();
            state_lck.status = format!("Recalled {} Band ! ! !", band);
        } else {
            state_lck.status = "Error with enable pin!".to_string();
        }
    Ok(())
    } else {
        Err(Box::new(Error::other("No Encoder Present")))
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
    if persist_current_profile(&mut state_lck, true).is_ok() {
        state_lck.status = "All data successfully saved !".to_string();
    }
}

fn write_atomic(path: &PathBuf, contents: &str) -> Result<(), String> {
    let tmp_path = path.with_extension(format!(
        "tmp-{}-{}",
        std::process::id(),
        chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default()
    ));
    fs::write(&tmp_path, contents).map_err(|e| e.to_string())?;
    fs::rename(&tmp_path, path).map_err(|e| {
        let _ = fs::remove_file(&tmp_path);
        e.to_string()
    })?;
    Ok(())
}

fn persist_current_profile(state_lck: &mut AppState, notify_meter: bool) -> Result<(), String> {
    let file_path = path::Path::new(&state_lck.file);
    let dir = env::current_dir().map_err(|e| e.to_string())?;
    let full_path = dir.join("static").join(file_path);
    if !fs::exists(&full_path).map_err(|e| e.to_string())? {
        fs::File::create(&full_path).map_err(|e| e.to_string())?;
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
    saved_state.tci_watchdog_secs = state_lck.tci_watchdog_secs;
    saved_state.cat_enabled = state_lck.cat_enabled;
    saved_state.cat_status = state_lck.cat_status.clone();
    saved_state.cat_watchdog_secs = state_lck.cat_watchdog_secs;
    saved_state.rigctld_host = state_lck.rigctld_host.clone();
    saved_state.rigctld_port = state_lck.rigctld_port;
    saved_state.rig_model_id = state_lck.rig_model_id;
    saved_state.rig_serial_device = state_lck.rig_serial_device.clone();
    saved_state.rig_baud = state_lck.rig_baud;
    saved_state.rig_civaddr = state_lck.rig_civaddr.clone();
    saved_state.rig_extra_conf = state_lck.rig_extra_conf.clone();
    saved_state.tune_reference_pin = state_lck.tune_reference_pin;
    saved_state.tune_reference_active_low = state_lck.tune_reference_active_low;
    let output_data = serde_json::to_string_pretty(&saved_state).map_err(|e| e.to_string())?;
    println!("Saving file to {}", full_path.to_string_lossy());
    write_atomic(&full_path, &output_data)?;
    if notify_meter {
        if let Some(tx) = state_lck.meter_sender.clone() {
            let _ = tx.send(true);
        }
    }
    Ok(())
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
    let stepper_lck = stepper.lock().unwrap();
    data.entry("max".to_string()).insert_entry(normalized_stepper_max(&stepper_lck) as u32);
    data.entry("pos".to_string()).insert_entry(stepper_lck.pos.load(Ordering::Relaxed) as u32);
    let mut temp_mem_data = HashMap::new();
    for (k, v) in stepper_lck.mem.clone() {
        temp_mem_data.entry(k).insert_entry(v.load(Ordering::Relaxed)as u32);
        
    }
    temp_mem_data
    
    }

fn default_profile_path() -> Result<PathBuf, std::io::Error> {
    Ok(env::current_dir()?.join("static").join("default_profile.txt"))
}

fn read_default_profile_name() -> Option<String> {
    let path = default_profile_path().ok()?;
    if let Ok(contents) = fs::read_to_string(path) {
        let name = contents.trim().to_string();
        if !name.is_empty() {
            return Some(name);
        }
    }
    None
}

fn write_default_profile_name(file_name: &str) -> Result<(), std::io::Error> {
    fs::write(default_profile_path()?, format!("{}\n", file_name))
}

fn clear_default_profile_name() -> Result<(), std::io::Error> {
    let path = default_profile_path()?;
    if path.exists() {
        fs::remove_file(path)?;
    }
    Ok(())
}

fn validate_profile(output: &StoredData) -> Result<(), String> {
    for (name, map) in [("tune", &output.tune), ("ind", &output.ind), ("load", &output.load)] {
        validate_stepper_profile(name, map)?;
    }
    for stepper in ["tune", "ind", "load"] {
        let mem = output
            .mem
            .get(stepper)
            .ok_or_else(|| format!("Profile missing memory map for {stepper}"))?;
        for band in ALL_BAND_KEYS {
            if !mem.contains_key(band) {
                return Err(format!("Profile missing {stepper} memory for {band}"));
            }
        }
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
    validate_profile(&output)?;
    apply_profile_to_state(state, file_name, output)
}

fn apply_profile_to_state(state: Arc<Mutex<AppState>>, file_name: &str, output: StoredData) -> Result<(), String> {
    let mut state_lck = state.lock().unwrap();
    state_lck.file = file_name.to_string();
    let mut my_stepper_arr = [
        state_lck.tune.clone(),
        state_lck.ind.clone(),
        state_lck.load.clone(),
    ];
    let my_output_arr = [&output.tune, &output.ind, &output.load];
    for (i, stepper) in my_stepper_arr.iter_mut().enumerate() {
        let name = &stepper.lock().unwrap().name.clone();
        if stepper.lock().unwrap().pin_a.unwrap_or(0u8) != 0 {
            handle_stepper(&mut state_lck, HashMap::new(), name, false, |_| stepper.clone());
        }
        wait_for_stepper_shutdown(stepper);
        let pin_a = my_output_arr[i].get("PinA").copied().map(|v| v as u8);
        let pin_b = my_output_arr[i].get("PinB").copied().map(|v| v as u8);
        let ena = my_output_arr[i].get("ena").copied().map(|v| v as u8);
        let stored_pos = *my_output_arr[i]
            .get("pos")
            .ok_or_else(|| format!("Profile missing {} position", stepper.lock().unwrap().name))? as i32;
        let mut normalized_max = (*my_output_arr[i]
            .get("max")
            .ok_or_else(|| format!("Profile missing {} max", stepper.lock().unwrap().name))? as i32)
            .max(stored_pos);
        if let Some(stepper_mem) = output.mem.get(&stepper.lock().unwrap().name) {
            for band in ALL_BAND_KEYS {
                let value = *stepper_mem.get(band).unwrap_or(&0) as i32;
                normalized_max = normalized_max.max(value);
            }
        }
        let ratio = *my_output_arr[i]
            .get("ratio")
            .ok_or_else(|| format!("Profile missing {} ratio", stepper.lock().unwrap().name))? as u8;
        stepper.lock().unwrap().max.store(normalized_max, Ordering::Relaxed);
        stepper.lock().unwrap().pos.store(stored_pos, Ordering::Relaxed);
        stepper.lock().unwrap().pin_a = pin_a;
        stepper.lock().unwrap().pin_b = pin_b;
        stepper.lock().unwrap().ena = ena;
        stepper.lock().unwrap().ratio = ratio;
        let mut stepper_lck = stepper.lock().unwrap();
        if stepper_lck.name == "ind" {
            println!("Inductor set to lower speed");
            stepper_lck.speed = Duration::from_micros(400);
        }
        if stepper_lck.pin_a.is_some() {
            stepper_lck.run_2()?;
        }
        drop(stepper_lck);
        for band in ALL_BAND_KEYS {
            let mut stepper_lck = stepper.lock().unwrap();
            let value = *output
                .mem
                .get(&stepper_lck.name)
                .ok_or_else(|| format!("Profile missing {} memory map", stepper_lck.name))?
                .get(band)
                .unwrap_or(&0) as i32;
            stepper_lck.mem.entry(band.to_string()).and_modify(|v| v.store(value, Ordering::Relaxed));
        }
    }
    state_lck.enc = if output.enc.contains_key("PinA") && output.enc.contains_key("PinB") {
        if let Some(enc) = &state_lck.enc {
            enc.stop();
            wait_for_encoder_shutdown(enc);
            println!("Deconfiguring Encoder to load new config");
        }
        Some(Encoder::new(
            *output.enc.get("PinA").ok_or_else(|| "Profile missing encoder PinA".to_string())? as u8,
            *output.enc.get("PinB").ok_or_else(|| "Profile missing encoder PinB".to_string())? as u8,
        ))
    } else {
        None
    };
    if let Some(mut enc) = state_lck.enc.clone() {
        enc.run()?;
    }
    state_lck.band = output.band;
    state_lck.call_sign = output.call_sign;
    let mut derived: HashMap<String, bool> = HashMap::new();
    for band in ALL_BAND_KEYS {
        let has_key =
            output.mem.get("tune").and_then(|m| m.get(band)).is_some()
            || output.mem.get("ind").and_then(|m| m.get(band)).is_some()
            || output.mem.get("load").and_then(|m| m.get(band)).is_some();
        let file_flag = output.mem_valid.get(band).copied().unwrap_or(false);
        derived.insert(band.to_string(), has_key || file_flag);
    }
    state_lck.mem_valid = derived;
    for key in ALL_BAND_KEYS {
        state_lck.mem_valid.entry(key.to_string()).or_insert(false);
    }
    if !output.tci_server.is_empty() {
        state_lck.tci_server = output.tci_server;
    }
    state_lck.follow_me = output.follow_me;
    state_lck.tci_watchdog_secs = output.tci_watchdog_secs.max(3);
    state_lck.cat_enabled = output.cat_enabled;
    state_lck.cat_watchdog_secs = output.cat_watchdog_secs.max(3);
    if !state_lck.follow_me {
        state_lck.pending_tci_band = None;
    }
    if !state_lck.cat_enabled {
        state_lck.pending_cat_band = None;
    }
    if state_lck.cat_enabled && state_lck.follow_me {
        state_lck.follow_me = false;
        state_lck.pending_tci_band = None;
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
    let mut tune_reference_pin = state_lck.tune_reference_pin;
    assign_optional_pin(
        &mut state_lck.gpio_pins,
        &mut tune_reference_pin,
        output.tune_reference_pin,
    )?;
    state_lck.tune_reference_pin = tune_reference_pin;
    state_lck.tune_reference_active_low = output.tune_reference_active_low;
    refresh_tune_reference_status(&mut state_lck);
    Ok(())
}

//processes all Multi-part form data for all post request handlers.
async fn process_form(mut form: Multipart) -> Result<HashMap<String, String>, String> {
    let mut form_data: HashMap<String, String> = HashMap::new();
    println!("Config PostForm Handler");
    while let Some(val) = form
        .next_field()
        .await
        .map_err(|err| format!("multipart field error: {}", err))?
    {
        let k = val
            .name()
            .map(str::to_string)
            .ok_or_else(|| "multipart field missing name".to_string())?;
        println!("Name: {:?}", k);
        let v = val
            .text()
            .await
            .map_err(|err| format!("multipart text error for {}: {}", k, err))?;
        println!("Key: {}, Value: {}", k, v);
        form_data.insert(k.clone(), v.clone());
    }
    println!("Pwr Button form data {:?}", form_data);
    Ok(form_data)
}

#[cfg(test)]
mod tests {
    use super::{Bands, StoredData, ALL_BAND_KEYS};

    #[test]
    fn test_profile_json_deserializes_and_is_consistent() {
        let profile: StoredData =
            serde_json::from_str(include_str!("../static/test.json")).expect("test.json must deserialize");

        for stepper in ["tune", "ind", "load"] {
            let bands = profile
                .mem
                .get(stepper)
                .unwrap_or_else(|| panic!("missing {stepper} memory map"));
            for band in ALL_BAND_KEYS {
                assert!(bands.contains_key(band), "{stepper} missing band {band}");
            }
        }

        for band in ALL_BAND_KEYS {
            assert_eq!(
                profile.mem_valid.get(band),
                Some(&true),
                "mem_valid should mark {band} as learned"
            );
        }

        let current_band = match profile.band {
            Bands::M10 => "10M",
            Bands::M11 => "11M",
            Bands::M15 => "15M",
            Bands::M20 => "20M",
            Bands::M40 => "40M",
            Bands::M80 => "80M",
        };

        assert_eq!(
            profile.mem["tune"][current_band],
            profile.tune["pos"],
            "tune position should match active band memory"
        );
        assert_eq!(
            profile.mem["ind"][current_band],
            profile.ind["pos"],
            "inductor position should match active band memory"
        );
        assert_eq!(
            profile.mem["load"][current_band],
            profile.load["pos"],
            "load position should match active band memory"
        );
    }
}
