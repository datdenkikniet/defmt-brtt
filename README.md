# `defmt-brtt`: defmt over `rtt` and `bbq`, simultaneously!

This crate combines the functionality of `defmt-rtt` and `defmt-bbq` into one single crate.

This allows you to retrieve data over RTT, but also to pump it out over some other transport by reading the data from a `bbqueue`. 

Even if that is not a use-case you're interested in, you can also `defmt-brtt`  as an easier way of switching between using RTT and/or BBQueue as your defmt transport.

# Features

* `rtt`: activate the RTT transport (works exactly like [`defmt-rtt`](https://docs.rs/defmt-rtt/0.4.0/defmt_rtt/), except for [Buffer Size](#buffer-size)).
* `bbq`: activate the BBQueue transport (works exactly like [`defmt-bbq`](https://docs.rs/defmt-bbq/0.1.0/defmt_bbq/), except for [Buffer Size](#buffer-size)).
* `async-await`: add a function to `defmt_brtt::DefmtConsumer` that enables async-waiting for log data.

You must activate at least one of `rtt` and `bbq`.

# Buffer Size
To configure the buffer size used by `defmt-brtt`, you can set the environment variable `DEFMT_BRTT_BUFFER_SIZE` to the desired size.

For example, if we'd want to use a 512 byte internal buffer, we would run `DEFMT_BRTT_BUFFER_SIZE=512 cargo build` in the build directory of a project.

Note that `defmt-brtt` assigned one buffer of size `DEFMT_BRTT_BUFFER_SIZE` once for both `rtt` and `bbq`, if those features are activated.


# User code required for use
To use the `defmt` logger implementation provided by `defmt-brtt`, you must always add insert the following use statement somewhere in your project:

```rust
use defmt_brtt as _;
```

You must also, somewhere in your code before calling any `defmt` logging function, call the `defmt_brtt::init!` macro.

```rust
fn main() {
    let _ = defmt_brtt::init!();
}
```

## `rtt`
To use the `rtt` functionality of this crate, you only need to do is activate the `rtt` feature (activated by default).

## `bbq`
To use the `bbq` functionality of this crate, you must activate the `bbq` feature (activated by default). You must call `defmt_brtt::init!` and use the returned `DefmtConsumer` to consume the `defmt` data.

The data that is produced by the `DefmtConsumer` can then be transported and fed to a decoder, such as [`defmt-print`](https://crates.io/crates/defmt-print), that will reconstruct the log messages from the `defmt` data.

```rust
fn main() {
    let logger = defmt_brtt::init();

    loop {
        if let Some(grant) = logger.read() {
            let written_bytes = write_my_log_data_over_usb(&grant).ok();
            // The step below is optional. Dropping the `Grant` releases
            // all read bytes.
            grant.release(written_bytes);
        }
    }
}

// If you have the `async-await` feature enabled, you
// can also do the following:
async fn read_logs(consumer: DefmtConsumer) {
    loop {
        let grant = logger.wait_for_log().await;
        let written_bytes = write_my_log_data_over_usb(&grant).ok();
        // The step below is optional. Dropping the `Grant` releases
        // all read bytes.
        grant.release(written_bytes);
    }
}
```