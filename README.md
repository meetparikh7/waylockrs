# waylockrs

waylockrs is a screen locking utility for Wayland compositors written in Rust.
It is compatible with any Wayland compositor which implements the
ext-session-lock-v1 Wayland protocol.

This project aims to replicate most of [swaylock]’s features in idiomatic Rust
to reduce risks like buffer overflows and authentication vulnerabilities. While
swaylock remains more battle-tested, this implementation draws heavily from its
design and credits the original authors for serving as a reference.

The default config, [defaults.toml](defaults.toml), has a list of options that
can be configured. These options can also be set via CLI arguments, as noted in
the file's comments. On first run, waylockrs will attempt to port the swaylock
config if found.


## Installation

TODO: packaging

### Compiling from Source

This package can be compiled with cargo, given the following system depedencies
are installed

* wayland
* wayland-protocols \*
* libxkbcommon
* cairo
* pam \*\*

_\* Compile-time dep_  \
_\*\* As of now, waylockrs requires pam and does not support suid_

Run these commands:

```sh
cargo build --release                            # Build the tool
sudo cp target/release/waylockrs /usr/local/bin  # Install the binary
sudo cp pam/waylockrs /etc/pam.d/waylockrs       # Copy the pam config file
```

[swaylock]: https://github.com/swaywm/swaylock
