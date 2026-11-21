fn main() {
    println!("cargo:rerun-if-changed=assets/kadr.ico");
    if std::env::var_os("CARGO_CFG_WINDOWS").is_some() {
        let mut res = winres::WindowsResource::new();
        res.set_icon("assets/kadr.ico");
        res.compile().expect("Failed to compile Windows icon resource");
    }
}
