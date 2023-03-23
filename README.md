# `defmt-brtt`: defmt over `rtt` and `bbq`, simultaneously!

This crate combines the functionality of `defmt-rtt` and `defmt-bbq` into one single crate.

This allows you to retrieve data over RTT, but also to pump it out over some other transport by reading the data from a `bbqueue`. 

Even if that is not a use-case you're interested in, you can also `defmt-brtt`  as an easier way of switching between using RTT and/or BBQueue as your defmt
transport.

# Features

* `rtt`: activate the RTT transport (works exactly like `defmt-rtt`).
* `bbq`: activate the BBQ transport (works exactly like `defmt-bbq`).
* `async-await`: add a function to `defmt_brtt::DefmtConsumer` that enables async-waiting for log data.

You must select at least one of `rtt` or `bbq`.