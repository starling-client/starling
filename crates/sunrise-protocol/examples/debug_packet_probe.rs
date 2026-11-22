use std::{error::Error, io::Write};
use sunrise_core::vanilla_bytes::Writer;
use sunrise_protocol::{
    buf::{Codec, Reader},
    packets::play::{DEBUG_SUBSCRIPTION_COUNT, DebugSample, DebugSubscriptionRequest},
};
fn main() -> Result<(), Box<dyn Error>> {
    let args: Vec<_> = std::env::args().collect();
    let mut out = std::fs::File::create(&args[2])?;
    for line in std::fs::read_to_string(&args[1])?.lines() {
        let input: serde_json::Value = serde_json::from_str(line)?;
        let result = match input["packet"].as_str().unwrap() {
            "registry" => serde_json::json!({"count": DEBUG_SUBSCRIPTION_COUNT}),
            // Compare the stream decoders independently of client handler validation.
            "decode_request" => {
                let body = unhex(input["body"].as_str().unwrap());
                let result = match DebugSubscriptionRequest::read(&mut Reader::new(&body)) {
                    Ok(p) => format!("ok {:?}", p.subscriptions),
                    Err(_) => "error".into(),
                };
                serde_json::json!({"result": result})
            }
            "decode_sample" => {
                let body = unhex(input["body"].as_str().unwrap());
                let result = match DebugSample::read(&mut Reader::new(&body)) {
                    Ok(p) => format!("ok {:?} {}", p.sample, p.kind),
                    Err(_) => "error".into(),
                };
                serde_json::json!({"result": result})
            }
            // Sunrise encodes the values vanilla encoded; the bytes must be vanilla's.
            kind => {
                let mut writer = Writer::new();
                if kind == "sample" {
                    let sample = serde_json::from_value(input["sample"].clone())?;
                    let kind = input["kind"].as_i64().unwrap() as i32;
                    DebugSample::write(&DebugSample { sample, kind }, &mut writer);
                } else {
                    let subscriptions = serde_json::from_value(input["ids"].clone())?;
                    DebugSubscriptionRequest::write(
                        &DebugSubscriptionRequest { subscriptions },
                        &mut writer,
                    );
                }
                let hex: String = writer.data.iter().map(|b| format!("{b:02x}")).collect();
                serde_json::json!({"wire": hex})
            }
        };
        writeln!(out, "{result}")?;
    }
    Ok(())
}
fn unhex(hex: &str) -> Vec<u8> {
    (0..hex.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&hex[i..i + 2], 16).unwrap())
        .collect()
}
