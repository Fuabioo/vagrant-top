# vagrant-top

A TUI for monitoring Vagrant virtual machines.

## Features

- **Environment Discovery**: Automatically detects all Vagrant environments from the machine-index
- **Table View**: Displays environments with aggregated metrics including CPU, memory, network I/O, and VM count
- **Chart View**: Visual representation of environment resource utilization
- **Flexible Sorting**: Sort environments by name, VM count, CPU, memory, network activity, disk I/O, uptime, and last change
- **Configurable Columns**: Toggle visibility of columns 1-7 to customize your view
- **Real-time Metrics**: Continuously polls virsh domstats for up-to-date VM statistics
- **Status Indicators**: Shows whether environments are running, partial, stopped, saved, or crashed
- **Uptime Tracking**: Reads real VM start times from libvirt PID files for accurate uptime display
- **State Change Detection**: Tracks VM state transitions with a dedicated LAST-CHG column

## Installation

### Homebrew

```bash
brew tap Fuabioo/tap
brew install vagrant-top
```

### Cargo

```bash
cargo install vagrant-top
```

### Binary Download

Download the archive for your platform from [GitHub releases](https://github.com/Fuabioo/vagrant-top/releases), extract it, and place `vagrant-top` in your `$PATH`.

## Usage

Simply run:

```bash
vagrant-top
```

The application auto-discovers Vagrant environments from your machine-index. No configuration needed.

## Keybindings

| Key | Action |
|-----|--------|
| `q` / `Ctrl+C` | Quit |
| `j` / `Down` | Next row |
| `k` / `Up` | Previous row |
| `g` | Jump to top |
| `G` | Jump to bottom |
| `Tab` | Toggle Table / Chart view |
| `1-7` | Toggle columns |
| `s` | Cycle sort column forward |
| `S` | Cycle sort column backward |
| `r` | Reverse sort direction |
| `?` | Toggle help overlay |

## Requirements

- Vagrant with libvirt/QEMU provider
- `virsh` CLI available in PATH
- Terminal with Nerd Font support for proper icon rendering

## License

MIT
