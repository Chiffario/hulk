# Hulk - small IPC-based CLI for gamma controls on Wayland

## Requirements
Hulk uses zwlr_gamma_control, which limits your choices to compositors using wl-roots, smithay, and some odd-ones-out like Hyprland

## Usage
To build, use `cargo build --release`
To run, run the resulting executable with flags `--gamma` and `--monitor` (or just use `--help`)

If you want daemon mode, just run the executable as `hulk && detach` or something to run it as a background process, and use the CLI with the normal flags to change settings

## TODO
- [ ] systemd unit
- [ ] color-management-v1 support but good
- [ ] proper documentation for the FIFO so that you can write there yourself (it uses RON but the message format is not stabilised, duh)
