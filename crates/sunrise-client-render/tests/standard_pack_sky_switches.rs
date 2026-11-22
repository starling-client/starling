#[path = "../fixtures/standard_pack_sky_switches.rs"]
mod fixture;

#[test]
#[ignore = "daily"]
fn standard_pack_sky_switches() {
    fixture::run_daily();
}
