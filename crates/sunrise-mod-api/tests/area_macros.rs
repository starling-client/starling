//! Public area macros must expand from outside the API crate, including recursion.

struct Host;
impl mod_api::Host for Host {}
mod_api::impl_host_areas!(Host);
impl mod_api::ServerHost for Host {}
mod_api::impl_server_host_areas!(Host);
impl mod_api::ClientHost for Host {}
mod_api::impl_client_host_areas!(Host);

mod_api::__skip_items!([blocks fluids] const INCLUDED: bool = true;);
mod_api::__skip_items!([blocks items] const INCLUDED: bool = false;);

#[test]
fn external_area_macros_implement_host_unions() {
    let _: &dyn mod_api::Host = &Host;
    let _: &dyn mod_api::ServerHost = &Host;
    let _: &dyn mod_api::ClientHost = &Host;
    assert!(std::hint::black_box(INCLUDED));
}

#[test]
fn client_quit_refuses_host_without_live_client() {
    let host: &dyn mod_api::ClientHost = &Host;
    assert!(!host.request_client_quit());
}
