# `defmt-brtt`: defmt over `rtt` and `bbq`, simultaneously!

This crate combines the functionality of `defmt-rtt` and `defmt-bbq` into one single crate.

This allows you to retrieve data over RTT, but also to pump it out over some other transport by reading the data from a `bbqueue`. 

Even if that is not a use-case you're interested in, you can also `defmt-brtt`  as an easier way of switching between using RTT and/or BBQueue as your defmt
transport.

# Features

* `rtt`: activate the RTT transport (works exactly like [`defmt-rtt`](https://docs.rs/defmt-rtt/0.4.0/defmt_rtt/)).
* `bbq`: activate the BBQueue transport (works exactly like [`defmt-bbq`](https://docs.rs/defmt-bbq/0.1.0/defmt_bbq/)).
* `async-await`: add a function to `defmt_brtt::DefmtConsumer` that enables async-waiting for log data.

You must select at least one of `rtt` and `bbq`.

# User code required for use
To use the `defmt` logger implementation provided by `defmt-brtt`, add the following use statement to your code:
```rust
use defmt_brtt as _;
```

## `rtt`
To use the `rtt` functionality of this crate, all you need to do is activate the `rtt` feature (enabled by default).

## `bbq`
To use the `bbq` functionality of this crate, you must activate the `bbq` feature (enabled by default), and you must call `defmt_brtt::init`