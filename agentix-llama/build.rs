fn main() {
    #[cfg(feature = "cuda")]
    {
        if let Ok(cuda_path) = std::env::var("CUDA_PATH") {
            println!("cargo:rustc-link-search=native={}/lib64", cuda_path);
            println!("cargo:rustc-link-lib=cudart");
            println!("cargo:rustc-link-lib=cublas");
        }
    }
}
