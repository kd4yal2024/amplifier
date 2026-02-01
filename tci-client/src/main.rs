use futures_util::StreamExt;
use tokio_tungstenite::tungstenite::Message;

fn split_frames(s: &str) -> impl Iterator<Item = &str> {
    // TCI frames may be separated by newline and/or ';'
    s.split(|c| c == ';' || c == '\n' || c == '\r')
        .map(str::trim)
        .filter(|f| !f.is_empty())
}

fn parse_any_tx_hz(frame: &str) -> Option<u64> {
    // Supports:
    //   tx_frequency:14170000
    //   tx_frequency_thetis:14170000,b20m,false,false
    let (k, v) = frame.split_once(':')?;
    let k = k.trim();

    if k == "tx_frequency" {
        return v.trim().parse::<u64>().ok();
    }

    if k == "tx_frequency_thetis" {
        let mut parts = v.split(',').map(str::trim);
        let hz: u64 = parts.next()?.parse().ok()?;
        return Some(hz);
    }

    None
}

fn band_from_hz(hz: u64) -> Option<&'static str> {
    match hz {
        // 80m: 3.5 - 4.0 MHz
        3_500_000..=4_000_000 => Some("80m"),

        // 40m: 7.0 - 7.3 MHz
        7_000_000..=7_300_000 => Some("40m"),

        // 20m: 14.0 - 14.35 MHz
        14_000_000..=14_350_000 => Some("20m"),

        // 15m: 21.0 - 21.45 MHz
        21_000_000..=21_450_000 => Some("15m"),

        // 11m: 26.0 - 27.999... MHz (anything >=26.0 and <28.0)
        26_000_000..=27_999_999 => Some("11m"),

        // 10m segments: 28.0 - 29.7 MHz
        28_000_000..=28_499_999 => Some("10m-1"),
        28_500_000..=28_999_999 => Some("10m-2"),
        29_000_000..=29_700_000 => Some("10m-3"),

        _ => None,
    }
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let ws_url = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "ws://192.168.0.108:50001".to_string());

    println!("Connecting to {ws_url} ...");
    let (mut ws, _resp) = tokio_tungstenite::connect_async(ws_url.as_str()).await?;
    println!("Connected. Listening... (Ctrl+C to quit)");

    let mut last_band: Option<&'static str> = None;

    while let Some(msg) = ws.next().await {
        match msg? {
            Message::Text(s) => {
                for frame in split_frames(&s) {
                    if let Some(hz) = parse_any_tx_hz(frame) {
                        if let Some(band) = band_from_hz(hz) {
                            if last_band != Some(band) {
                                last_band = Some(band);
                                println!("BAND_CHANGE band={} hz={}", band, hz);
                            }
                        }
                    }
                }
            }
            Message::Close(c) => {
                println!("Closed: {c:?}");
                break;
            }
            _ => {}
        }
    }

    Ok(())
}
