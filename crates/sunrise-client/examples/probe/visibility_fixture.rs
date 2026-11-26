//! Transport for the vanilla 26.3 section visibility parity gate (`tools/test_visibility_parity.py`).
use std::path::Path;

pub fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<_> = crate::args().skip(1).collect();
    let [witness] = args.as_slice() else {
        return Err("usage: visibility_fixture WITNESS_JSON".into());
    };
    let checked = sunrise_client::gfx::visibility_probe::verify_witness(Path::new(witness))?;
    println!("Section visibility parity passed ({checked} cases against {witness})");
    Ok(())
}
