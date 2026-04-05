pub mod encoder {
    use rppal::gpio::{Gpio, Level};
    use rppal::system::DeviceInfo;
    use std::sync::{
        Arc,
        atomic::{AtomicBool, AtomicI32, Ordering},
    };
    use std::thread;
    use std::time::Duration;
    extern crate rppal;

    
    /*
       fn main() {
           let mut my_enc = Encoder::new();
           match  &mut my_enc.run() {
               Ok(_) => {}
               e => {
                   eprintln!("Error: {:?}", e);
               }
           }
       }
    */
    #[derive(Clone)]
    pub struct Encoder {
        pub pin_a: u8,
        pub pin_b: u8,
        pub stop: Arc<AtomicBool>,
        pub running: Arc<AtomicBool>,
        pub count: Arc<AtomicI32>,
    }
    impl Encoder {
        pub fn new(pina: u8, pinb: u8) -> Self {
            Self {
                pin_a: pina,
                pin_b: pinb,
                stop: Arc::new(AtomicBool::new(false)),
                running: Arc::new(AtomicBool::new(false)),
                count: Arc::new(AtomicI32::new(0)),
            }
        }
        pub fn run(&mut self) -> Result<(), String> {
            let device_info = DeviceInfo::new()
                .map_err(|e| format!("Encoder device info failed: {e}"))?;
            println!(
                "Model: {} (SoC: {})",
                device_info.model(),
                device_info.soc()
            );
            if self.running.swap(true, Ordering::Relaxed) {
                return Ok(());
            }
            self.stop.store(false, Ordering::Relaxed);
            let master_count = Arc::clone(&self.count);

            let pin_a = self.pin_a;
            let pin_b = self.pin_b;
            let stop = self.stop.clone();
            let running = self.running.clone();
            thread::spawn(move || {
                let gpio = match Gpio::new() {
                    Ok(gpio) => gpio,
                    Err(err) => {
                        eprintln!("Encoder GPIO init failed: {err}");
                        running.store(false, Ordering::Relaxed);
                        return;
                    }
                };
                let pin1 = match gpio.get(pin_a) {
                    Ok(pin) => pin.into_input_pullup(),
                    Err(err) => {
                        eprintln!("Encoder pin A init failed on GPIO {pin_a}: {err}");
                        running.store(false, Ordering::Relaxed);
                        return;
                    }
                };
                let pin2 = match gpio.get(pin_b) {
                    Ok(pin) => pin.into_input_pullup(),
                    Err(err) => {
                        eprintln!("Encoder pin B init failed on GPIO {pin_b}: {err}");
                        running.store(false, Ordering::Relaxed);
                        return;
                    }
                };

                let mut last_clk_state = Level::High;
                loop {
                    if stop.load(Ordering::Relaxed) {
                        break;
                    }
                    let state = pin1.read();
                    match state {
                        Level::High => {
                            if last_clk_state == Level::Low {
                                if let Level::Low = pin2.read() {
                                    //tx.send(1).unwrap();
                                    master_count.fetch_add(1, Ordering::Relaxed);
                                } else {
                                    //tx.send(-1).unwrap();
                                    master_count.fetch_add(-1, Ordering::Relaxed);
                                }

                                last_clk_state = Level::High;
                            }
                        }

                        Level::Low => {
                            last_clk_state = state;
                        }
                    }
                    thread::sleep(Duration::from_micros(10));
                }
                running.store(false, Ordering::Relaxed);
            });
            /*
                        let mut  count = 0;
                        for received in rx {
                            count += received;

                            println!("Got: {} for a count of {}", received, count);

                        }

            */
            Ok(())
        }
        pub fn enc(&self) -> i32 {
            self.count.load(Ordering::Relaxed)
        }

        pub fn stop(&self) {
            self.stop.store(true, Ordering::Relaxed);
        }

        pub fn is_running(&self) -> bool {
            self.running.load(Ordering::Relaxed)
        }
    }
}

pub mod stepper {
    use std::sync::Mutex;
    use std::sync::{Arc, mpsc::{self, Sender}, atomic::{AtomicBool, AtomicI32, Ordering}};
    use rppal::gpio::Gpio;
    use std::collections::HashMap;
    use std::thread;
    use std::time::Duration;

    #[derive(Clone)]
    pub struct Stepper {
        pub name: String,
        pub channel: Option<Sender<(u32, bool)>>,
        pub pin_a: Option<u8>,
        pub pin_b: Option<u8>,
        pub ena: Option<u8>,
        pub ratio: u8,
        pub pos: Arc<AtomicI32>,
        pub mem: HashMap<String, Arc<AtomicI32>>,
        pub max: Arc<AtomicI32>,
        pub speed: Duration,
        pub operate: Arc<Mutex<bool>>,
        pub running: Arc<AtomicBool>,
    }
    impl Stepper {
        pub fn new(name: &str) -> Self {
            Self {
                name: name.to_string(),
                channel: None,
                pin_a: None,
                pin_b: None,
                ena: None,
                ratio: 1,
                pos: Arc::new(AtomicI32::new(0)),
                mem: HashMap::from([
                    ("10M".to_string(), Arc::new(AtomicI32::new(0))),
                    ("11M".to_string(), Arc::new(AtomicI32::new(0))),
                    ("15M".to_string(), Arc::new(AtomicI32::new(0))),
                    ("20M".to_string(), Arc::new(AtomicI32::new(0))),
                    ("40M".to_string(), Arc::new(AtomicI32::new(0))),
                    ("80M".to_string(), Arc::new(AtomicI32::new(0))),
                ]),
                max: Arc::new(AtomicI32::new(100000)),
                speed: Duration::from_micros(100),
                operate: Arc::new(Mutex::new(false)),
                running: Arc::new(AtomicBool::new(false)),
            }
        }
        pub fn run(&self, val: u32) -> Result<(), String> {
            let pos: u32 = self.pos.load(Ordering::Relaxed) as u32;
            let pin_a = self.pin_a.ok_or_else(|| format!("{} missing step pin", self.name))?;
            let pin_b = self.pin_b.ok_or_else(|| format!("{} missing direction pin", self.name))?;
            let gpio = Gpio::new().map_err(|e| format!("{} GPIO init failed: {e}", self.name))?;
            let mut pulse_pin = gpio
                .get(pin_a)
                .map_err(|e| format!("{} pulse pin {pin_a} init failed: {e}", self.name))?
                .into_output();
            let mut dir_pin = gpio
                .get(pin_b)
                .map_err(|e| format!("{} direction pin {pin_b} init failed: {e}", self.name))?
                .into_output();
            let mut count = 0;
            pulse_pin.set_low();
            if val > pos {
                dir_pin.set_high();
                while val > self.pos.load(Ordering::Relaxed) as u32 {
                    count += 1;
                    pulse_pin.set_high();
                    thread::sleep(self.speed);
                    pulse_pin.set_low();
                    thread::sleep(self.speed);
                    if count % 2 == 0 { 
                        let _ = self.pos.fetch_update(Ordering::Relaxed, Ordering::Relaxed, |current| Some(current.saturating_add(1)));
                    }
                }
            } else if val < pos {
                dir_pin.set_low();
                while val < self.pos.load(Ordering::Relaxed) as u32 {
                    count += 1;
                    pulse_pin.set_high();
                    thread::sleep(self.speed);
                    pulse_pin.set_low();
                    thread::sleep(self.speed);
                    if count % 2 == 0 {
                        let _ = self.pos.fetch_update(Ordering::Relaxed, Ordering::Relaxed, |current| Some(current.saturating_sub(1)));
                    }
                }
            }
            Ok(())
        }

        pub fn run_2(&mut self) -> Result<(), String> {
            println!("Inside run 2");
            if self.running.load(Ordering::Relaxed) {
                return Ok(());
            }
            let (tx, rx) = mpsc::channel();
            self.channel = Some(tx);
            let pin_a = self.pin_a.ok_or_else(|| format!("{} missing step pin", self.name))?;
            let pin_b = self.pin_b.ok_or_else(|| format!("{} missing direction pin", self.name))?;
            let gpio = Gpio::new().map_err(|e| format!("{} GPIO init failed: {e}", self.name))?;
            let mut pulse_pin = gpio
                .get(pin_a)
                .map_err(|e| format!("{} pulse pin {pin_a} init failed: {e}", self.name))?
                .into_output();
            let mut dir_pin = gpio
                .get(pin_b)
                .map_err(|e| format!("{} direction pin {pin_b} init failed: {e}", self.name))?
                .into_output();
            let mut count = 0;
            let pos = self.pos.clone();
            let speed = self.speed;
            let operate = self.operate.clone();
            let running = self.running.clone();
            running.store(true, Ordering::Relaxed);
            thread::spawn(move ||  {
                loop{
                    if let Ok((val, stop))  = rx.recv() {
                        if stop {
                            println!("Stopping stepper loop to delete stepper.");
                            break;
                        }
                        pulse_pin.set_low();
                        if val > pos.load(Ordering::Relaxed) as u32 {
                            *operate.lock().unwrap() = true;
                            dir_pin.set_high();
                            while val > pos.load(Ordering::Relaxed) as u32 {
                                count += 1;
                                pulse_pin.set_high();
                                thread::sleep(speed);
                                pulse_pin.set_low();
                                thread::sleep(speed);
                                if count % 2 == 0 { 
                                    let _ = pos.fetch_update(Ordering::Relaxed, Ordering::Relaxed, |current| Some(current.saturating_add(1)));
                                }
                            } 
                            *operate.lock().unwrap() = false;
                        } else if val < pos.load(Ordering::Relaxed) as u32{
                            *operate.lock().unwrap() = true;
                            dir_pin.set_low();
                            while val < pos.load(Ordering::Relaxed) as u32 {
                                count += 1;
                                pulse_pin.set_high();
                                thread::sleep(speed);
                                pulse_pin.set_low();
                                thread::sleep(speed);
                                if count % 2 == 0 {
                                    let _ = pos.fetch_update(Ordering::Relaxed, Ordering::Relaxed, |current| Some(current.saturating_sub(1)));
                                }
                            }
                            *operate.lock().unwrap() = false;
                        }
                        
                    }
                }
                *operate.lock().unwrap() = false;
                running.store(false, Ordering::Relaxed);
            });
            Ok(())
        }

        pub fn stop(&mut self) {
            if let Some(tx) = self.channel.clone() {
                let _ = tx.send((self.pos.load(Ordering::Relaxed) as u32, true));
            }
            self.channel = None;
        }

        pub fn is_running(&self) -> bool {
            self.running.load(Ordering::Relaxed)
        }
    }
}
pub mod mcp {
    use std::collections::HashMap;
    //use linux_embedded_hal::I2cdev;
    use mcp230xx::{Direction, Mcp230xx, Mcp23017, Level};
    use rppal::{self, {i2c::I2c}};
    use std::sync::{Arc, Mutex};
    use embedded_devices::devices::texas_instruments::ina228::{INA228Sync, address::{Address, Pin}};
    use embedded_devices::sensor::OneshotSensorSync;
    use uom::si::electric_current::{ampere, milliampere};
    use uom::si::electric_potential::volt;
    use uom::si::electrical_resistance::ohm;
    use uom::si::f64::{ElectricCurrent, ElectricalResistance};
    use uom::si::thermodynamic_temperature::degree_celsius;
    use embedded_hal::delay::DelayNs;
    use std::time::Duration;  
    use embedded_interfaces::i2c::I2cDeviceSync;
    use embedded_hal_bus::i2c::MutexDevice;
    use embedded_hal_compat::ReverseCompat; 
      #[derive(Clone, Copy, Debug, Default)]
    pub struct StdDelay;

    impl DelayNs for StdDelay {
        fn delay_ns(&mut self, ns: u32) {
            // Good enough for device init delays on Linux
            std::thread::sleep(Duration::from_nanos(ns as u64));
        }

        fn delay_us(&mut self, us: u32) {
            std::thread::sleep(Duration::from_micros(us as u64));
        }

        fn delay_ms(&mut self, ms: u32) {
            std::thread::sleep(Duration::from_millis(ms as u64));
        }
    } 
    #[derive(Clone)] 
    pub struct Mcp {
        pub all_pins: [Mcp23017; 16],
        pub pins: HashMap<String, Mcp23017>,
        pub bus: Arc<Mutex<I2c>>,
        pub message: String,
        pub switch: HashMap<String, String>
    }
    impl Mcp {
        // default function that sets all pins as output.
        pub fn new() -> Result<Self, String> {
            //let i2c= I2cdev::new("/dev/i2c-1").unwrap();
            let all_pins = [
                Mcp23017::A0, Mcp23017::A1, Mcp23017::A2,
                Mcp23017::A3, Mcp23017::A4, Mcp23017::A5,
                Mcp23017::A6, Mcp23017::A7, Mcp23017::B0,
                Mcp23017::B1, Mcp23017::B2, Mcp23017::B3,
                Mcp23017::B4, Mcp23017::B5, Mcp23017::B6,
                Mcp23017::B7,
            ];
           
            Ok(Self {
                all_pins,
                bus: Arc::new(Mutex::new(I2c::new().map_err(|e| format!("I2C bus init failed: {e}"))?)),
                pins: HashMap::from([
                    ("A0".to_string(), Mcp23017::A0),
                    ("A1".to_string(), Mcp23017::A1),
                    ("A2".to_string(), Mcp23017::A2),
                    ("A3".to_string(), Mcp23017::A3),
                    ("A4".to_string(), Mcp23017::A4),
                    ("A5".to_string(), Mcp23017::A5),
                    ("A6".to_string(), Mcp23017::A6),
                    ("A7".to_string(), Mcp23017::A7),
                    ("B0".to_string(), Mcp23017::B0),
                    ("B1".to_string(), Mcp23017::B1),
                    ("B2".to_string(), Mcp23017::B2),
                    ("B3".to_string(), Mcp23017::B3),
                    ("B4".to_string(), Mcp23017::B4),
                    ("B5".to_string(), Mcp23017::B5),
                    ("B6".to_string(), Mcp23017::B6),
                    ("B7".to_string(), Mcp23017::B7),
                ]),
                message: String::from("MCP Intioalized ! ! !"),
                switch: HashMap::new(),
                })
                
        }
        pub fn init(&mut self) -> Result<(), String> {
            let i2c_mcp = MutexDevice::new(&self.bus).reverse();
            let mut mcp: Mcp230xx<_, Mcp23017> =
                Mcp230xx::new(i2c_mcp, 0x20).map_err(|e| format!("MCP23017 init failed: {e:?}"))?;
             for i in 0..=15 {
                let pin = Mcp23017::try_from(i).map_err(|_| format!("Invalid MCP pin index {i}"))?;
                println!("{:?}", pin);
                if mcp.set_direction(pin, Direction::Output).is_ok() {
                    println!("Pin: {:?} Configured as output", pin);
                }
                let _ = mcp.set_gpio(self.all_pins[i], Level::Low);
            }
            Ok(())
        }
        pub fn read_pin(&mut self, pin: Mcp23017)-> Result<Level, String> {
            let i2c_mcp = MutexDevice::new(&self.bus).reverse();
            let mut mcp: Mcp230xx<_, Mcp23017> =
                Mcp230xx::new(i2c_mcp, 0x20).map_err(|e| format!("MCP23017 open failed: {e:?}"))?;
            mcp.gpio(pin).map_err(|e| format!("MCP23017 read {pin:?} failed: {e:?}"))
        }
        pub fn set_pin(&mut self, pin: Mcp23017, val: Level)-> Result<(), String>{
            let i2c_mcp = MutexDevice::new(&self.bus).reverse();
            let mut mcp: Mcp230xx<_, Mcp23017> =
                Mcp230xx::new(i2c_mcp, 0x20).map_err(|e| format!("MCP23017 open failed: {e:?}"))?;
            mcp.set_gpio(pin, val)
                .map_err(|e| format!("MCP23017 write {pin:?} failed: {e:?}"))?;
            Ok(())

        }
        pub fn read_val(&self) -> Result<(f64, f64, f64), String> {
            let i2c_ina = MutexDevice::new(&self.bus);
            let delay = StdDelay;
            let mut ina: INA228Sync<StdDelay, I2cDeviceSync<MutexDevice<'_, _>, u8>> = INA228Sync::new_i2c(delay, i2c_ina, Address::A0A1(Pin::Gnd, Pin::Gnd));
            ina.init(
                ElectricalResistance::new::<ohm>(0.015),
                ElectricCurrent::new::<ampere>(3.0),
            )
            .map_err(|e| format!("INA228 init failed: {e:?}"))?;
            let val = ina
                .measure()
                .map_err(|e| format!("INA228 measurement failed: {e:?}"))?;
            let temp = val.temperature.get::<degree_celsius>();
            let current = val.current.get::<milliampere>();
            let voltage = val.bus_voltage.get::<volt>();
            Ok((temp, current, voltage))
            
        }
    }
}
    
 
