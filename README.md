# `defmt-brtt`

`defmt-brtt` sends the same defmt byte stream over RTT and an in-memory
[`bbqueue`](https://docs.rs/bbqueue/0.7), depending on the enabled features.
The queue can be drained by an application-specific transport such as USB or a
network interface.

## Features

- `rtt`: enable the RTT transport. Enabled by default.
- `bbq`: enable the synchronous BBQueue transport. Enabled by default.
- `async-await`: enable BBQueue's native async notifier and
  [`DefmtConsumer::wait_for_log`]. This feature implies `bbq`.
- `portable-atomic-critical-section`: provide async atomics on targets without
  native compare-and-swap instructions, such as Cortex-M0. Enable this together
  with `async-await` unless the firmware selects another portable-atomic backend.

At least one of `rtt` and `bbq` must be enabled.

## Encoding

The firmware must select the defmt encoding. Using rzCOBS is strongly
recommended because the BBQueue transport discards remaining bytes when the
queue is full. rzCOBS framing lets the decoder recover after dropped data.

```toml
[dependencies]
defmt = { version = "1", features = ["encoding-rzcobs"] }
defmt-brtt = "0.1"
```

Raw encoding is only safe when the application can guarantee that the queue
never fills.

## Buffer Size

Set `DEFMT_BRTT_BUFFER_SIZE` while compiling to change the default 1024-byte
buffer:

```console
DEFMT_BRTT_BUFFER_SIZE=512 cargo build
```

Each enabled transport allocates a separate buffer of this size.

## Initialization

Link the global logger and initialize it before emitting any defmt log:

```rust
use defmt_brtt as _;

fn main() {
    let consumer = defmt_brtt::init!().unwrap();
    // Pass `consumer` to the task responsible for forwarding logs.
}
```

Calling `init!()` more than once returns `InitError::AlreadyInitialized`.
Logging before initialization permanently latches an initialization error.
With only `rtt` enabled, `init!()` is a no-op returning `Result<(), ()>`.

## Synchronous Consumption

```rust
fn forward_logs(mut consumer: defmt_brtt::DefmtConsumer) -> ! {
    loop {
        if let Ok(grant) = consumer.read() {
            let written = write_my_log_data(&grant).unwrap_or(0);
            grant.release(written);
        }
    }
}
```

`read()` returns one contiguous section. Read again after releasing it to
drain data that wrapped around the queue. Dropping a grant releases zero bytes;
always call `release` with the number of bytes successfully forwarded.

## Asynchronous Consumption

Enable `async-await` and await BBQueue's native notification:

```rust
async fn forward_logs(mut consumer: defmt_brtt::DefmtConsumer) -> ! {
    loop {
        let grant = consumer.wait_for_log().await;
        let written = write_my_log_data(&grant).await.unwrap_or(0);
        grant.release(written);
    }
}
```

The queue is single-producer, single-consumer. Reading and waiting require
mutable access to the consumer, preventing concurrent waits or reads while a
wait is pending. Release or drop each grant before reading or waiting again;
BBQueue permits only one outstanding read grant. Encoded chunks become visible
as they are written; consumers must treat the queue as a byte stream rather than
one grant per defmt frame.
