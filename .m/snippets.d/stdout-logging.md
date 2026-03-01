# stdout vs logging

stdout is reserved for in-band output the agent reads (hook context, setup/teardown instructions, distill output, etc.).
All other output — status messages, confirmations, diagnostics — must use `tracing` crate logging (`info!`, `warn!`,
etc.), which goes to stderr.
