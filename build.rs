use std::{env, path::PathBuf};

fn main() {
    println!("cargo:rerun-if-env-changed=DEFMT_BRTT_BUFFER_SIZE");

    let size = env::var("DEFMT_BRTT_BUFFER_SIZE")
        .map(|s| {
            s.parse()
                .expect("could not parse DEFMT_BRTT_BUFFER_SIZE as usize")
        })
        .unwrap_or(1024_usize);

    let out_dir_path = PathBuf::from(env::var_os("OUT_DIR").unwrap());
    let out_file_path = out_dir_path.join("consts.rs");

    std::fs::write(
        out_file_path,
        format!(
            "/// BBQ & RTT buffer size (default: 1024. Consumed twice, once for RTT and once for BBQueue).
            ///
            /// Can be customized by setting the `DEFMT_BRTT_BUFFER_SIZE` environment variable.
            /// Use a power of 2 for best performance.
            pub const BUF_SIZE: usize = {};",
            size
        ),
    )
    .unwrap();
}
