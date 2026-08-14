fn main() {
    #[cfg(feature = "cuda")]
    {
        println!("cargo:rustc-env=GGML_CUDA=1");
    }
}
