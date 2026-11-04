fn main() {
    #[cfg(windows)]
    {
        println!("cargo:rerun-if-changed=gnobbi.rc");
        println!("cargo:rerun-if-changed=../../packaging/windows/gnobbi.ico");
        embed_resource::compile("gnobbi.rc", embed_resource::NONE).manifest_optional().unwrap();
    }
}
