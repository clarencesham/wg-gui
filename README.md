# wg-gui

##### a GUI client on Ubuntu Linux 24.04 for [Wireguard](https://www.wireguard.com/)

________________
Features:

- System tray icon: black when disconnected, red when any tunnel is connected;
  menu with Show / Quit and a Tunnels submenu (checkbox per tunnel).

- Tunnel list from /etc/wireguard: status icon per tunnel (green/gray) and an
  optional country flag of each tunnel's endpoint (offline mmdb lookup,
  database downloaded automatically and refreshed weekly).

- Activate / deactivate tunnels with wg-quick up / wg-quick down.

- Per-peer details: public key, allowed IPs, endpoint, latest handshake and
  transfer stats (received/sent); activate or deactivate individual peers on
  the running interface without editing the config.

- Tunnel management:
  - add tunnels (single .conf file or many at once from a .zip archive)
  - delete tunnels (with confirmation)
  - edit tunnels (rename + text editor with syntax coloring)
  - export all tunnels into a single .zip file

- Settings:
  - allow multiple active tunnels at the same time
  - start hidden in the system tray
  - check for updates on start
  - show country flags next to tunnel names
  - remember connected servers and reconnect them on start
  - passwordless polkit policy (no password prompt on start)

- App behavior:
  - single instance (launching it again just shows the existing window)
  - F5 rescans /etc/wireguard
  - window auto-fits to show all peers of the selected tunnel
  - built-in log pane (connection attempts and errors)

- Desktop integration:
  - runs as root via pkexec (polkit policy with GUI support)
  - .desktop launcher + icon installable via install.sh (pinnable to the
    Ubuntu dock / application grid)


## Clone

```sh
git clone https://github.com/clarencesham/wg-gui.git
```

## Install

```sh
cd wg-gui
sudo ./install.sh
```

## Build and Test

```sh
cd wg-gui
. "$HOME/.cargo/env"		# rustup was installed to ~/.cargo

cargo build			# debug binary: target/debug/wg-gui
cargo build --release		# optimized:	target/release/wg-gui
```

```sh
cargo test			# run tests (GTK tests skip without a display)

sudo apt install xvfb		# install once to run GTK tests headless
xvfb-run -a cargo test -- --test-threads=1	# run all tests, including GTK UI/glade tests
```

## Run

Open wg-gui from the Applications menu. This launches it with pkexec and
provides a graphical authentication prompt.

To launch from a terminal in your logged-in desktop session:

```sh
pkexec ./target/release/wg-gui
```

If the tray reports that `dbus-launch` is missing, install it with:

```sh
sudo apt install dbus-x11
```
