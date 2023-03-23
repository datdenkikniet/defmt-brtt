# `defmt-brtt`: defmt over `rtt` and `bbq`, simultaneously!

This crate combines the functionality of `defmt-rtt` and `defmt-bbq` into one single crate.

This allows you to retrieve data over RTT, but also to pump it out over some other transport by reading the data from a `bbqueue`. Even if those
things are not of particularly much use to you, you can also simply use it as an easier way of switching between using RTT and/or BBQueue as your defmt
transport.