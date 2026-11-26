//! The game `tools/mod_pipeline_bench.py` measures: Sunrise with the benchmark mod
//! (`tools/fixtures/mod-pipeline-bench`) linked in, or with no mod at all when
//! `SUNRISE_PIPELINE_BENCH=off`, so both rows run the same binary.
#[path = "../../../tools/fixtures/mod-pipeline-bench/src/lib.rs"]
mod bench;

use mod_api::__private::native::BuiltMod;
use mod_api::{ModEntry, Side};

static BUILT: [BuiltMod; 1] = [BuiltMod {
    id: bench::ID,
    version: bench::VERSION,
    side: Side::Client,
}];

fn main() {
    let startup = sunrise_client::entry::prepare();
    if std::env::var_os("SUNRISE_PIPELINE_BENCH").is_some_and(|value| value == "off") {
        sunrise_mod_host::main_with(Vec::new(), &[], startup);
    } else {
        sunrise_mod_host::main_with(
            vec![ModEntry::new(bench::ID, Box::<bench::Bench>::default())],
            &BUILT,
            startup,
        );
    }
}
