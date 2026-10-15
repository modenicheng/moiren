//! cargo run --locked -p moiren-app --features gpui-test --example ui_capture
fn main() -> anyhow::Result<()> {
    moiren_app::ui::run_ui_capture()
}
