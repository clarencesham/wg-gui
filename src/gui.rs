use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::path::Path;
use std::rc::Rc;
use std::sync::{Arc, Mutex};
use std::thread;

use gtk::glib;
use gtk::prelude::*;

use crate::conf::Conf;
use crate::countries;
use crate::humanize;
use crate::settings::Settings;
use crate::tunnels::{self, PeerDisplay, TUNNELS_PATH};
use crate::util;
use crate::wg;

pub const VERSION: &str = "1.2.0";
pub const REPO: &str = "https://github.com/UnnoTed/wg-gui";

pub enum Msg {
    ShowWindow,
    TunnelToggled {
        name: String,
        error: Option<String>,
        from_tray: bool,
    },
    PeerToggled {
        tunnel: String,
        public_key: String,
        activate: bool,
        error: Option<String>,
    },
    CountryFlag {
        tunnel: String,
        code: String,
    },
    UpdateAvailable {
        tag: String,
    },
    Log {
        level: String,
        text: String,
    },
    Error(String),
    Rescan,
}

pub struct PeerRow {
    public_key: String,
    active: Rc<Cell<bool>>,
    handshake: gtk::Label,
    transfer: gtk::Label,
    endpoint: gtk::Label,
    button: gtk::Button,
}

pub struct App {
    pub app: gtk::Application,
    pub window: gtk::ApplicationWindow,
    pub header: gtk::HeaderBar,
    pub settings: Arc<Mutex<Settings>>,
    pub tx: glib::Sender<Msg>,
    pub indicator: RefCell<Option<libappindicator::AppIndicator>>,
    pub tray_menu: gtk::Menu,
    pub menu_tunnels: gtk::MenuItem,

    pub list: gtk::ListBox,
    pub peer_list: gtk::Box,
    pub peer_scroller: gtk::ScrolledWindow,
    pub peer_rows: RefCell<Vec<Rc<PeerRow>>>,
    pub peer_frames: RefCell<Vec<gtk::Frame>>,

    pub iface_status: gtk::Label,
    pub iface_public_key: gtk::Label,
    pub iface_listen_port: gtk::Label,
    pub iface_addresses: gtk::Label,
    pub iface_dns: gtk::Label,
    pub button_change_state: gtk::Button,
    pub button_settings: gtk::Button,
    pub button_quit: gtk::Button,
    pub button_add_tunnel: gtk::Button,
    pub button_del_tunnel: gtk::Button,
    pub button_zip_tunnel: gtk::Button,
    pub button_edit_tunnel: gtk::Button,

    pub editor_window: gtk::Window,
    pub editor_name: gtk::Entry,
    pub editor_publickey: gtk::Entry,
    pub editor_text: gtk::TextView,
    pub editor_button_save: gtk::Button,
    pub editor_button_cancel: gtk::Button,
    pub editor_modified: Cell<bool>,

    pub settings_window: gtk::Window,
    pub settings_multiple: gtk::CheckButton,
    pub settings_start_tray: gtk::CheckButton,
    pub settings_check_updates: gtk::CheckButton,
    pub settings_country_flags: gtk::CheckButton,
    pub settings_remember: gtk::CheckButton,
    pub settings_polkit: gtk::CheckButton,
    pub settings_button_save: gtk::Button,
    pub settings_button_cancel: gtk::Button,

    pub logs: gtk::ListBox,

    pub icons: RefCell<HashMap<String, gtk::Image>>,
    pub flags: RefCell<HashMap<String, gtk::Image>>,
    pub tray_items: RefCell<HashMap<String, (gtk::CheckMenuItem, glib::SignalHandlerId)>>,

    pub gray_icon: gtk::gdk_pixbuf::Pixbuf,
    pub green_icon: gtk::gdk_pixbuf::Pixbuf,

    pub last_selected: RefCell<String>,
    pub peer_endpoints: Arc<Mutex<HashMap<String, String>>>,
}

fn get<T: IsA<glib::Object>>(builder: &gtk::Builder, id: &str) -> Result<T, String> {
    builder
        .object::<T>(id)
        .ok_or_else(|| format!("cant get object: {}", id))
}

fn short_peer_key(public_key: &str) -> String {
    if public_key.len() > 12 {
        format!("{}…", &public_key[..12])
    } else {
        public_key.to_string()
    }
}

fn colorize_conf(conf: &str) -> String {
    let property_color = "purple";
    let section_color = "green";

    let pairs: [(&str, String); 13] = [
        ("&", "&amp;".to_string()),
        ("PrivateKey", span(property_color, "PrivateKey")),
        ("PublicKey", span(property_color, "PublicKey")),
        ("Address", span(property_color, "Address")),
        ("DNS", span(property_color, "DNS")),
        ("AllowedIPs", span(property_color, "AllowedIPs")),
        ("Endpoint", span(property_color, "Endpoint")),
        ("PostUp", span(property_color, "PostUp")),
        ("PreDown", span(property_color, "PreDown")),
        ("PreSharedKey", span(property_color, "PreSharedKey")),
        (
            "PersistentKeepalive",
            span(property_color, "PersistentKeepalive"),
        ),
        ("[Interface]", span(section_color, "[Interface]")),
        ("[Peer]", span(section_color, "[Peer]")),
    ];

    let mut out = String::with_capacity(conf.len() * 2);
    let mut rest = conf;

    'outer: while !rest.is_empty() {
        for (pattern, replacement) in &pairs {
            if let Some(stripped) = rest.strip_prefix(pattern) {
                out.push_str(replacement);
                rest = stripped;
                continue 'outer;
            }
        }

        let mut chars = rest.chars();
        if let Some(c) = chars.next() {
            out.push(c);
            rest = chars.as_str();
        }
    }

    out
}

fn span(color: &str, text: &str) -> String {
    format!("<span color=\"{}\">{}</span>", color, text)
}

fn find_public_key(data: &str) -> Option<String> {
    for line in data.lines() {
        let trimmed = line.trim_start();
        if let Some(rest) = trimmed.strip_prefix("PublicKey") {
            let rest = rest.trim_start();
            if let Some(value) = rest.strip_prefix('=') {
                return Some(value.trim().to_string());
            }
        }
    }

    None
}

pub fn latest_release_tag() -> Result<String, String> {
    let response = ureq::get("https://api.github.com/repos/UnnoTed/wg-gui/releases")
        .set("User-Agent", "wg-gui")
        .call()
        .map_err(|e| e.to_string())?;

    let body = response.into_string().map_err(|e| e.to_string())?;
    let json: serde_json::Value = serde_json::from_str(&body).map_err(|e| e.to_string())?;

    json.as_array()
        .and_then(|releases| releases.first())
        .and_then(|release| release.get("tag_name"))
        .and_then(|tag| tag.as_str())
        .map(|tag| tag.to_string())
        .ok_or_else(|| "no releases found".to_string())
}

impl App {
    pub fn create(
        application: &gtk::Application,
        builder: &gtk::Builder,
        window: &gtk::ApplicationWindow,
        indicator: Option<libappindicator::AppIndicator>,
        tray_menu: gtk::Menu,
        settings: Arc<Mutex<Settings>>,
        tx: glib::Sender<Msg>,
    ) -> Result<Rc<App>, String> {
        let header = get::<gtk::HeaderBar>(builder, "main_header")?;

        // build a dynamic list of peers instead of the single static one
        let box_peers = get::<gtk::Box>(builder, "box_peers")?;
        for child in box_peers.children() {
            box_peers.remove(&child);
        }

        let peer_scroller =
            gtk::ScrolledWindow::new(gtk::Adjustment::NONE, gtk::Adjustment::NONE);
        peer_scroller.set_policy(gtk::PolicyType::Never, gtk::PolicyType::Automatic);

        let peer_viewport = gtk::Viewport::new(gtk::Adjustment::NONE, gtk::Adjustment::NONE);
        let peer_list = gtk::Box::new(gtk::Orientation::Vertical, 8);
        peer_list.set_hexpand(true);

        peer_viewport.add(&peer_list);
        peer_scroller.add(&peer_viewport);
        box_peers.set_homogeneous(false);
        box_peers.set_vexpand(true);
        box_peers.pack_start(&peer_scroller, true, true, 0);
        peer_scroller.show_all();

        let gray_icon = util::load_embedded_pixbuf("Icon/not_connected.png")?;
        let green_icon = util::load_embedded_pixbuf("Icon/connected.png")?;

        let app = Rc::new(App {
            app: application.clone(),
            window: window.clone(),
            header,
            settings,
            tx,
            indicator: RefCell::new(indicator),
            tray_menu,
            menu_tunnels: gtk::MenuItem::with_label("Tunnels"),

            list: get::<gtk::ListBox>(builder, "tunnel_list")?,
            peer_list,
            peer_scroller,
            peer_rows: RefCell::new(Vec::new()),
            peer_frames: RefCell::new(Vec::new()),

            iface_status: get::<gtk::Label>(builder, "label_interface_status")?,
            iface_public_key: get::<gtk::Label>(builder, "label_interface_public_key")?,
            iface_listen_port: get::<gtk::Label>(builder, "label_interface_listen_port")?,
            iface_addresses: get::<gtk::Label>(builder, "label_interface_addresses")?,
            iface_dns: get::<gtk::Label>(builder, "label_interface_dns_servers")?,
            button_change_state: get::<gtk::Button>(builder, "button_change_state")?,
            button_settings: get::<gtk::Button>(builder, "button_settings")?,
            button_quit: get::<gtk::Button>(builder, "button_quit")?,
            button_add_tunnel: get::<gtk::Button>(builder, "button_add_tunnel")?,
            button_del_tunnel: get::<gtk::Button>(builder, "button_del_tunnel")?,
            button_zip_tunnel: get::<gtk::Button>(builder, "button_zip_tunnel")?,
            button_edit_tunnel: get::<gtk::Button>(builder, "button_edit_tunnel")?,

            editor_window: get::<gtk::Window>(builder, "editor_window")?,
            editor_name: get::<gtk::Entry>(builder, "editor_name")?,
            editor_publickey: get::<gtk::Entry>(builder, "editor_publickey")?,
            editor_text: get::<gtk::TextView>(builder, "editor_text")?,
            editor_button_save: get::<gtk::Button>(builder, "editor_button_save")?,
            editor_button_cancel: get::<gtk::Button>(builder, "editor_button_cancel")?,
            editor_modified: Cell::new(false),

            settings_window: get::<gtk::Window>(builder, "settings_window")?,
            settings_multiple: get::<gtk::CheckButton>(
                builder,
                "settings_multiple_active_tunnels",
            )?,
            settings_start_tray: get::<gtk::CheckButton>(builder, "settings_start_tray")?,
            settings_check_updates: get::<gtk::CheckButton>(builder, "settings_check_updates")?,
            settings_country_flags: get::<gtk::CheckButton>(builder, "settings_country_flags")?,
            settings_remember: get::<gtk::CheckButton>(builder, "settings_remember_connected")?,
            settings_polkit: get::<gtk::CheckButton>(builder, "settings_polkit_policy")?,
            settings_button_save: get::<gtk::Button>(builder, "button_settings_save")?,
            settings_button_cancel: get::<gtk::Button>(builder, "button_settings_cancel")?,

            logs: get::<gtk::ListBox>(builder, "wireguard_logs")?,

            icons: RefCell::new(HashMap::new()),
            flags: RefCell::new(HashMap::new()),
            tray_items: RefCell::new(HashMap::new()),

            gray_icon,
            green_icon,

            last_selected: RefCell::new(String::new()),
            peer_endpoints: Arc::new(Mutex::new(HashMap::new())),
        });

        // load the static header icons from the embedded assets
        for (id, path) in [
            ("icon_delete", "Icon/delete.png"),
            ("icon_tunnel", "Icon/add_tunnel.png"),
            ("icon_zip", "Icon/zip.png"),
        ] {
            let pixbuf = util::load_embedded_pixbuf(path)?;
            let image = get::<gtk::Image>(builder, id)?;
            image.set_from_pixbuf(Some(&pixbuf));
        }

        let version_label = get::<gtk::Label>(builder, "label_version")?;
        version_label.set_text(&format!("VERSION: v{}", VERSION));
        version_label.set_tooltip_text(Some(REPO));

        app.wire()?;

        Ok(app)
    }

    fn wire(self: &Rc<Self>) -> Result<(), String> {
        self.tray_menu.append(&self.menu_tunnels);
        self.tray_menu.reorder_child(&self.menu_tunnels, 1);
        self.tray_menu.show_all();

        self.set_indicator_icon("wireguard_off");

        let devices = wg::devices()?;
        for device in &devices {
            self.header
                .set_subtitle(Some(&format!("Connected to {}", device.name)));
            self.set_indicator_icon("wg_connected");
        }

        {
            let app = self.app.clone();
            self.button_quit.connect_clicked(move |_| app.quit());
        }

        {
            let a = self.clone();
            self.button_settings.connect_clicked(move |_| {
                if let Err(e) = a.from_settings() {
                    a.show_error(&e);
                }
                a.settings_window.set_title("wg-gui - Settings");
                a.settings_window.show_all();
            });
        }

        {
            let a = self.clone();
            self.button_change_state.connect_clicked(move |_| {
                let row = a.list.selected_row();
                a.toggle_tunnel(row.as_ref(), "", false);
            });
        }

        {
            let a = self.clone();
            self.list.connect_row_selected(move |_, row| {
                if let Some(row) = row {
                    a.update_row(row);
                }
            });
        }

        self.wire_add_tunnel();
        self.wire_delete_tunnel();
        self.wire_zip_tunnel();
        self.wire_editor();

        {
            let a = self.clone();
            self.editor_window
                .connect_hide(move |_| {
                    if let Err(e) = a.scan_tunnels() {
                        a.show_error(&e);
                    }
                    a.editor_modified.set(false);
                });
        }

        self.wire_settings();

        // the windows' close buttons only hide them, so provide explicit
        // close/quit buttons where needed
        for window in [&self.editor_window, &self.settings_window] {
            window.connect_delete_event(|window, _| {
                window.hide();
                glib::Propagation::Stop
            });
        }

        self.window.connect_delete_event(|window, _| {
            window.hide();
            glib::Propagation::Stop
        });

        {
            let a = self.clone();
            self.window.connect_key_press_event(move |_, event| {
                if event.keyval() == gtk::gdk::keys::constants::F5 {
                    a.wlog("INFO", "Scanning tunnels from keyboard command (F5)");
                    if let Err(e) = a.scan_tunnels() {
                        a.wlog("ERROR", &e);
                    }
                }

                glib::Propagation::Proceed
            });
        }

        self.from_settings()?;

        {
            let a = self.clone();
            glib::timeout_add_seconds_local(1, move || {
                a.tick();
                glib::ControlFlow::Continue
            });
        }

        // reconnect the servers of the previous run
        {
            let settings = self.settings.clone();
            let tx = self.tx.clone();
            thread::spawn(move || {
                let servers: Vec<(String, i64)> = {
                    let current = settings.lock().unwrap();
                    current
                        .connected_servers
                        .iter()
                        .map(|(name, time)| (name.clone(), *time))
                        .collect()
                };

                if settings.lock().unwrap().remember_connected_servers && !servers.is_empty() {
                    let mut sorted = servers;
                    sorted.sort_by_key(|(_, time)| *time);

                    for (tunnel, _) in sorted {
                        if let Err(e) = tunnels::toggle_tunnel(&tunnel, &settings) {
                            eprintln!("error connecting to tunnel {}: {}", tunnel, e);
                            let _ = tx.send(Msg::Log {
                                level: "ERROR".to_string(),
                                text: "error connecting to tunnel".to_string(),
                            });
                        }
                    }
                }

                tunnels::RECONNECTED.store(true, std::sync::atomic::Ordering::SeqCst);
                let _ = tx.send(Msg::Rescan);
            });
        }

        if self.settings.lock().unwrap().check_updates {
            let tx = self.tx.clone();
            thread::spawn(move || {
                thread::sleep(std::time::Duration::from_secs(60));

                match latest_release_tag() {
                    Ok(tag) if tag != format!("v{}", VERSION) => {
                        let _ = tx.send(Msg::UpdateAvailable { tag });
                    }
                    Ok(_) => {}
                    Err(e) => eprintln!("error on update check: {}", e),
                }
            });
        }

        if self.settings.lock().unwrap().country_flags {
            let tx = self.tx.clone();
            thread::spawn(move || {
                if let Err(e) = countries::open_database() {
                    eprintln!("error getting country database: {}", e);
                    let _ = tx.send(Msg::Log {
                        level: "ERROR".to_string(),
                        text: format!("error getting country database: {}", e),
                    });
                }
            });
        }

        self.wlog("INFO", &format!("wg-gui v{} started", VERSION));

        Ok(())
    }

    fn wire_add_tunnel(self: &Rc<Self>) {
        let a = self.clone();
        self.button_add_tunnel.connect_clicked(move |_| {
            let dialog = gtk::FileChooserDialog::with_buttons(
                Some("wg-gui - Choose tunnel files (*.conf, *.zip)"),
                Some(&a.window),
                gtk::FileChooserAction::Open,
                &[
                    ("OK", gtk::ResponseType::Accept),
                    ("Cancel", gtk::ResponseType::Cancel),
                ],
            );

            let result = (|| -> Result<(), String> {
                let filter = gtk::FileFilter::new();
                filter.add_pattern("*.conf");
                filter.add_pattern("*.zip");
                filter.set_name(Some("*.conf / *.zip"));

                dialog.add_filter(filter);
                dialog.set_select_multiple(true);

                if dialog.run() == gtk::ResponseType::Accept {
                    for path in dialog.filenames() {
                        let file_name = path.to_string_lossy().into_owned();

                        if file_name.ends_with(".zip") {
                            tunnels::parse_zip_archive(&file_name)?;
                            continue;
                        }

                        let data = std::fs::read(&path).map_err(|e| e.to_string())?;
                        let base = path
                            .file_name()
                            .map(|n| n.to_string_lossy().into_owned())
                            .unwrap_or_default();
                        std::fs::write(format!("{}{}", TUNNELS_PATH, base), data)
                            .map_err(|e| e.to_string())?;
                    }

                    a.scan_tunnels()?;
                }

                Ok(())
            })();

            util::destroy(&dialog);

            if let Err(e) = result {
                a.show_error(&e);
            }
        });
    }

    fn wire_delete_tunnel(self: &Rc<Self>) {
        let a = self.clone();
        self.button_del_tunnel.connect_clicked(move |_| {
            let result = (|| -> Result<(), String> {
                let row = a
                    .list
                    .selected_row()
                    .ok_or_else(|| "No row selected.".to_string())?;
                let name = row.widget_name().to_string();
                let path = tunnels::tunnel_path(&name);

                let dialog = gtk::MessageDialog::new(
                    Some(&a.window),
                    gtk::DialogFlags::MODAL,
                    gtk::MessageType::Question,
                    gtk::ButtonsType::YesNo,
                    &format!("Do you really want to delete {}?", name),
                );
                let response = dialog.run();
                util::destroy(&dialog);

                if response == gtk::ResponseType::Yes {
                    std::fs::remove_file(&path).map_err(|e| e.to_string())?;
                    a.scan_tunnels()?;
                }

                Ok(())
            })();

            if let Err(e) = result {
                a.show_error(&e);
            }
        });
    }

    fn wire_zip_tunnel(self: &Rc<Self>) {
        let a = self.clone();
        self.button_zip_tunnel.connect_clicked(move |_| {
            let dialog = gtk::FileChooserDialog::with_buttons(
                Some("wg-gui - zip tunnels -> Save zip file"),
                Some(&a.window),
                gtk::FileChooserAction::Save,
                &[
                    ("Cancel", gtk::ResponseType::Cancel),
                    ("Save", gtk::ResponseType::Accept),
                ],
            );

            dialog.set_do_overwrite_confirmation(true);

            const MONTHS: [&str; 12] = [
                "January",
                "February",
                "March",
                "April",
                "May",
                "June",
                "July",
                "August",
                "September",
                "October",
                "November",
                "December",
            ];

            if let Ok(now) = glib::DateTime::now_local() {
                let month = (now.month().clamp(1, 12) - 1) as usize;
                dialog.set_current_name(&format!(
                    "wg_tunnels_{}_{}_{}.zip",
                    now.day_of_month(),
                    MONTHS[month],
                    now.year()
                ));
            }

            let result = (|| -> Result<(), String> {
                if dialog.run() == gtk::ResponseType::Accept {
                    let dest = dialog
                        .filename()
                        .ok_or_else(|| "no destination selected".to_string())?;
                    tunnels::zip_tunnels(&dest.to_string_lossy())?;
                }

                Ok(())
            })();

            util::destroy(&dialog);

            if let Err(e) = result {
                a.show_error(&e);
            }
        });
    }

    fn wire_editor(self: &Rc<Self>) {
        {
            let a = self.clone();
            self.editor_name
                .connect_changed(move |_| a.editor_modified.set(true));
        }

        if let Some(buffer) = self.editor_text.buffer() {
            let a = self.clone();
            buffer.connect_changed(move |_| a.editor_modified.set(true));
        }

        {
            let a = self.clone();
            self.button_edit_tunnel.connect_clicked(move |_| {
                a.editor_modified.set(false);

                let result = (|| -> Result<(), String> {
                    let row = match a.list.selected_row() {
                        Some(row) => row,
                        None => return Ok(()),
                    };

                    let name = row.widget_name().to_string();
                    let path = tunnels::tunnel_path(&name);

                    a.editor_name.set_text(&name);
                    a.editor_window
                        .set_title(&format!("Edit tunnel - {}", name));

                    let data = std::fs::read_to_string(&path).map_err(|e| e.to_string())?;

                    // gets public key to use in the PublicKey entry
                    if let Some(public_key) = find_public_key(&data) {
                        a.editor_publickey.set_text(&public_key);
                    }

                    let markup = colorize_conf(&data);
                    let buffer = a
                        .editor_text
                        .buffer()
                        .ok_or_else(|| "cant get editor buffer".to_string())?;

                    buffer.set_text("");
                    buffer.insert_markup(&mut buffer.start_iter(), &markup);

                    a.editor_window.show_all();

                    Ok(())
                })();

                if let Err(e) = result {
                    a.show_error(&e);
                }
            });
        }

        {
            let a = self.clone();
            self.editor_button_cancel.connect_clicked(move |_| {
                if !a.editor_modified.get() {
                    a.editor_window.close();
                    return;
                }

                let dialog = gtk::MessageDialog::new(
                    Some(&a.editor_window),
                    gtk::DialogFlags::MODAL,
                    gtk::MessageType::Question,
                    gtk::ButtonsType::YesNo,
                    "Do you want to cancel any modification done to the tunnel?",
                );
                let response = dialog.run();
                util::destroy(&dialog);

                if response == gtk::ResponseType::Yes {
                    a.editor_modified.set(false);
                    a.editor_window.close();
                }
            });
        }

        {
            let a = self.clone();
            self.editor_button_save.connect_clicked(move |_| {
                let result = (|| -> Result<(), String> {
                    let row = match a.list.selected_row() {
                        Some(row) => row,
                        None => return Ok(()),
                    };

                    let name = row.widget_name().to_string();
                    let mut path = tunnels::tunnel_path(&name);

                    // get the new tunnel name
                    let new_name = a.editor_name.text().trim().to_string();

                    // rename the tunnel
                    if name != new_name {
                        let new_path = tunnels::tunnel_path(&new_name);
                        std::fs::rename(&path, &new_path).map_err(|e| e.to_string())?;
                        path = new_path;
                    }

                    // get the tunnels' edited text through editor_text's buffer
                    let buffer = a
                        .editor_text
                        .buffer()
                        .ok_or_else(|| "cant get editor buffer".to_string())?;
                    let data = buffer
                        .text(&buffer.start_iter(), &buffer.end_iter(), false)
                        .map(|text| text.to_string())
                        .unwrap_or_default();

                    std::fs::write(&path, data.as_bytes()).map_err(|e| e.to_string())?;

                    a.editor_modified.set(false);
                    a.scan_tunnels()?;
                    a.editor_window.close();

                    Ok(())
                })();

                if let Err(e) = result {
                    a.show_error(&e);
                }
            });
        }
    }

    fn wire_settings(self: &Rc<Self>) {
        {
            let a = self.clone();
            self.settings_button_save.connect_clicked(move |_| {
                let prev_country_flags = a.settings.lock().unwrap().country_flags;

                let result = (|| -> Result<(), String> {
                    a.to_settings();
                    a.settings.lock().unwrap().save()?;
                    a.settings_window.close();

                    // only download country database when previously disabled
                    // and now enabled
                    if !prev_country_flags && a.settings.lock().unwrap().country_flags {
                        let tx = a.tx.clone();
                        thread::spawn(move || {
                            if let Err(e) = countries::open_database() {
                                let _ = tx.send(Msg::Error(format!(
                                    "download country database error: {}",
                                    e
                                )));
                            }
                        });
                    }

                    a.set_passwordless()?;
                    a.scan_tunnels()?;

                    Ok(())
                })();

                if let Err(e) = result {
                    a.show_error(&e);
                }
            });
        }

        {
            let a = self.clone();
            self.settings_button_cancel.connect_clicked(move |_| {
                let loaded = Settings::load();
                *a.settings.lock().unwrap() = loaded;

                if let Err(e) = a.from_settings() {
                    a.show_error(&e);
                }

                a.settings_window.close();
            });
        }
    }

    pub fn scan_tunnels(self: &Rc<Self>) -> Result<(), String> {
        let mut config_by_name: HashMap<String, String> = HashMap::new();
        let mut config_list: Vec<String> = Vec::new();

        let entries = std::fs::read_dir(TUNNELS_PATH).map_err(|e| e.to_string())?;
        for entry in entries.flatten() {
            let path = entry.path();
            if !path.is_file() {
                continue;
            }

            let file_name = entry.file_name().to_string_lossy().into_owned();
            if !file_name.ends_with(".conf") {
                continue;
            }

            let name = file_name.trim_end_matches(".conf").to_string();
            config_by_name.insert(name.clone(), format!("{}{}", TUNNELS_PATH, file_name));
            config_list.push(name);
        }

        config_list.sort();

        while let Some(row) = self.list.row_at_index(0) {
            self.list.remove(&row);
        }

        let active_names = tunnels::active_device_names();
        if active_names.is_empty() {
            self.header.set_subtitle(Some("No active tunnels"));
        } else {
            self.header
                .set_subtitle(Some(&format!("Connected to {}", active_names.join(", "))));
        }

        let tunnels_menu = gtk::Menu::new();
        self.icons.borrow_mut().clear();
        self.flags.borrow_mut().clear();
        self.tray_items.borrow_mut().clear();

        let country_flags = self.settings.lock().unwrap().country_flags;
        let last_index = config_list.len().saturating_sub(1);

        for (i, name) in config_list.iter().enumerate() {
            let row = gtk::ListBoxRow::new();
            row.set_widget_name(name);
            row.set_margin_start(8);
            row.set_margin_end(8);
            if i == 0 {
                row.set_margin_top(8);
            } else if i == last_index {
                row.set_margin_bottom(8);
            }

            let menu_item = gtk::CheckMenuItem::with_label(name);
            tunnels_menu.append(&menu_item);

            let a = self.clone();
            let tunnel_name = name.clone();
            let handle = menu_item.connect_toggled(move |_| {
                a.toggle_tunnel(None, &tunnel_name, true);
            });

            let is_active = active_names.contains(name);
            menu_item.block_signal(&handle);
            menu_item.set_active(is_active);
            menu_item.unblock_signal(&handle);
            self.tray_items
                .borrow_mut()
                .insert(name.clone(), (menu_item.clone(), handle));

            let icon = gtk::Image::from_pixbuf(Some(if is_active {
                &self.green_icon
            } else {
                &self.gray_icon
            }));
            icon.set_valign(gtk::Align::Center);
            icon.set_halign(gtk::Align::Start);
            icon.set_size_request(10, 10);
            icon.set_vexpand(false);
            icon.set_hexpand(false);
            self.icons.borrow_mut().insert(name.clone(), icon.clone());

            let mut country: Option<gtk::Image> = None;
            if country_flags {
                let flag = util::load_embedded_image("flags/IDK.png")?;
                flag.set_valign(gtk::Align::Center);
                flag.set_halign(gtk::Align::Start);
                flag.set_size_request(10, 10);
                flag.set_vexpand(false);
                flag.set_hexpand(false);
                self.flags.borrow_mut().insert(name.clone(), flag.clone());
                country = Some(flag);

                let tx = self.tx.clone();
                let tunnel = name.clone();
                let path = config_by_name.get(name).cloned().unwrap_or_default();
                thread::spawn(move || {
                    let result = (|| -> Result<String, String> {
                        let text = std::fs::read_to_string(&path).map_err(|e| e.to_string())?;
                        let conf = Conf::parse(&text);
                        let ip = tunnels::endpoint_ip(&conf)?;
                        countries::find(&ip)
                    })();

                    match result {
                        Ok(code) => {
                            let _ = tx.send(Msg::CountryFlag { tunnel, code });
                        }
                        Err(e) => {
                            let _ = tx.send(Msg::Log {
                                level: "ERROR".to_string(),
                                text: format!("country flag error: {}", e),
                            });
                        }
                    }
                });
            }

            let label = gtk::Label::new(Some(name));
            label.set_halign(gtk::Align::Start);
            label.set_margin_start(8);
            label.set_margin_end(8);
            label.set_margin_top(8);
            label.set_margin_bottom(8);

            let row_box = gtk::Box::new(gtk::Orientation::Horizontal, 4);
            row_box.add(&icon);
            if let Some(country) = &country {
                row_box.add(country);
            }
            row_box.add(&label);

            row.add(&row_box);
            row.show_all();
            self.list.insert(&row, -1);

            if *self.last_selected.borrow() == *name {
                self.list.select_row(Some(&row));
            }
        }

        if self.last_selected.borrow().is_empty() {
            if let Some(row) = self.list.row_at_index(0) {
                self.list.select_row(Some(&row));
            }
        }

        self.menu_tunnels.set_submenu(Some(&tunnels_menu));
        self.tray_menu.show_all();

        Ok(())
    }

    pub fn update_row(self: &Rc<Self>, row: &gtk::ListBoxRow) {
        let result = (|| -> Result<(), String> {
            let devices = wg::devices()?;
            let id = row.widget_name().to_string();
            let path = tunnels::tunnel_path(&id);

            let text = std::fs::read_to_string(&path).map_err(|e| e.to_string())?;
            let conf = Conf::parse(&text);

            *self.last_selected.borrow_mut() = id.clone();
            self.unknown_labels();

            let iface = conf.interface();

            self.iface_addresses
                .set_text(iface.and_then(|s| s.get("Address")).unwrap_or(""));
            self.iface_status.set_text("Inactive");
            self.iface_dns
                .set_text(iface.and_then(|s| s.get("DNS")).unwrap_or(""));
            self.button_change_state.set_label("Activate");

            let device = devices.iter().find(|d| d.name == id);

            if let Some(device) = device {
                self.iface_status.set_text("Active");
                self.button_change_state.set_label("Deactivate");
                self.iface_public_key.set_text(&device.public_key);
                self.iface_listen_port
                    .set_text(&device.listen_port.to_string());
            }

            let peers = tunnels::peer_displays(&conf, device);
            self.render_peers(&id, device.is_some(), peers);
            self.schedule_fit_window();

            Ok(())
        })();

        if let Err(e) = result {
            self.show_error(&e);
        }
    }

    fn new_peer_frame(
        self: &Rc<Self>,
        tunnel: &str,
        device_active: bool,
        peer: PeerDisplay,
    ) -> Result<(gtk::Frame, PeerRow), String> {
        let title = if peer.comment.trim().is_empty() {
            "Peer".to_string()
        } else {
            peer.comment.clone()
        };

        let frame = gtk::Frame::new(Some(&title));
        frame.set_label_align(0.0, 0.5);
        frame.set_shadow_type(gtk::ShadowType::Out);

        let grid = gtk::Grid::new();
        grid.set_column_spacing(8);
        grid.set_row_spacing(4);
        grid.set_hexpand(true);

        let active = Rc::new(Cell::new(peer.active));

        let entries: [(&str, &str); 5] = [
            ("Public key:", &peer.public_key),
            ("Allowed IPs:", &peer.allowed_ips),
            ("Endpoint:", &peer.endpoint),
            ("Latest handshake:", &peer.handshake),
            ("Transfer:", &peer.transfer),
        ];

        let mut value_labels: Vec<gtk::Label> = Vec::new();

        for (i, (name, value)) in entries.iter().enumerate() {
            let name_label = gtk::Label::new(Some(name));
            name_label.set_halign(gtk::Align::End);

            let value_label = gtk::Label::new(Some(value));
            value_label.set_halign(gtk::Align::Start);
            value_label.set_hexpand(true);
            value_label.set_selectable(true);

            grid.attach(&name_label, 0, i as i32, 1, 1);
            grid.attach(&value_label, 1, i as i32, 1, 1);

            value_labels.push(value_label);
        }

        let button = gtk::Button::with_label("Deactivate");
        button.set_halign(gtk::Align::End);
        button.set_valign(gtk::Align::Center);
        button.set_margin_top(4);

        if !device_active {
            button.set_label("Activate");
            button.set_sensitive(false);
            button.set_tooltip_text(Some("The tunnel's interface is not active"));
        } else if peer.active {
            button.set_tooltip_text(Some("Remove this peer from the running interface"));
        } else {
            button.set_label("Activate");
            button.set_tooltip_text(Some("Add this peer back to the running interface"));
        }

        {
            let a = self.clone();
            let tunnel = tunnel.to_string();
            let public_key = peer.public_key.clone();
            let active = active.clone();
            let button_ref = button.clone();

            button.connect_clicked(move |_| {
                button_ref.set_sensitive(false);
                a.toggle_peer(&tunnel, &public_key, !active.get());
            });
        }

        grid.attach(&button, 0, entries.len() as i32, 2, 1);
        frame.add(&grid);

        let row = PeerRow {
            public_key: peer.public_key.clone(),
            active,
            handshake: value_labels[3].clone(),
            transfer: value_labels[4].clone(),
            endpoint: value_labels[2].clone(),
            button,
        };

        Ok((frame, row))
    }

    // rebuilds the peers list with one frame per peer
    fn render_peers(
        self: &Rc<Self>,
        tunnel: &str,
        device_active: bool,
        peers: Vec<PeerDisplay>,
    ) {
        let old_frames: Vec<gtk::Frame> = self.peer_frames.borrow_mut().drain(..).collect();
        for frame in old_frames {
            util::destroy(&frame);
        }
        self.peer_rows.borrow_mut().clear();

        for peer in peers {
            match self.new_peer_frame(tunnel, device_active, peer) {
                Ok((frame, row)) => {
                    frame.set_hexpand(true);
                    self.peer_list.pack_start(&frame, true, true, 0);
                    self.peer_frames.borrow_mut().push(frame);
                    self.peer_rows.borrow_mut().push(Rc::new(row));
                }
                Err(e) => eprintln!("error building peer frame: {}", e),
            }
        }

        self.peer_list.show_all();
    }

    // reloads a tunnel's config and live device and redraws its peers
    pub fn refresh_peers(self: &Rc<Self>, tunnel: &str) -> Result<(), String> {
        let text =
            std::fs::read_to_string(tunnels::tunnel_path(tunnel)).map_err(|e| e.to_string())?;
        let conf = Conf::parse(&text);

        let devices = wg::devices()?;
        let device = devices.iter().find(|d| d.name == tunnel);

        self.render_peers(tunnel, device.is_some(), tunnels::peer_displays(&conf, device));
        self.schedule_fit_window();

        Ok(())
    }

    pub fn toggle_tunnel(self: &Rc<Self>, row: Option<&gtk::ListBoxRow>, name: &str, from_tray: bool) {
        let name = if !name.is_empty() {
            name.to_string()
        } else {
            row.map(|r| r.widget_name().to_string()).unwrap_or_default()
        };

        if name.is_empty() {
            return;
        }

        let settings = self.settings.clone();
        let tx = self.tx.clone();

        thread::spawn(move || {
            let result = tunnels::toggle_tunnel(&name, &settings);
            let _ = tx.send(Msg::TunnelToggled {
                name,
                error: result.err(),
                from_tray,
            });
        });
    }

    pub fn toggle_peer(self: &Rc<Self>, tunnel: &str, public_key: &str, activate: bool) {
        let tunnel = tunnel.to_string();
        let public_key = public_key.to_string();
        let tx = self.tx.clone();
        let peer_endpoints = self.peer_endpoints.clone();

        thread::spawn(move || {
            let result = if activate {
                tunnels::activate_peer(&tunnel, &public_key, &peer_endpoints)
            } else {
                tunnels::remember_peer_endpoint(&tunnel, &public_key, &peer_endpoints);
                tunnels::deactivate_peer(&tunnel, &public_key)
            };

            let _ = tx.send(Msg::PeerToggled {
                tunnel,
                public_key,
                activate,
                error: result.err(),
            });
        });
    }

    pub fn handle_msg(self: &Rc<Self>, msg: Msg) {
        match msg {
            Msg::ShowWindow => {
                self.window.show();
                self.window.present();
            }
            Msg::TunnelToggled {
                name,
                error,
                from_tray,
            } => {
                match &error {
                    Some(e) => {
                        if from_tray {
                            self.wlog("ERROR", &format!("connect tunnel error: {}", e));
                        } else {
                            self.show_error(e);
                        }
                    }
                    None => {
                        if !name.is_empty() {
                            let active = tunnels::active_device_names();
                            if active.contains(&name) {
                                self.wlog("INFO", &format!("Connected to {}", name));
                            } else {
                                self.wlog("INFO", &format!("Disconnected from {}", name));
                            }
                        }
                    }
                }

                if let Err(e) = self.scan_tunnels() {
                    self.show_error(&e);
                }

                let active = tunnels::active_device_names();
                self.set_indicator_icon(if active.is_empty() {
                    "wireguard_off"
                } else {
                    "wg_connected"
                });
            }
            Msg::PeerToggled {
                tunnel,
                public_key,
                activate,
                error,
            } => {
                match &error {
                    Some(e) => self.show_error(e),
                    None => {
                        if activate {
                            self.wlog(
                                "INFO",
                                &format!("Activated peer {} on {}", short_peer_key(&public_key), tunnel),
                            );
                        } else {
                            self.wlog(
                                "INFO",
                                &format!(
                                    "Deactivated peer {} on {}",
                                    short_peer_key(&public_key),
                                    tunnel
                                ),
                            );
                        }
                    }
                }

                if let Err(e) = self.refresh_peers(&tunnel) {
                    eprintln!("error refreshing peers: {}", e);
                }
            }
            Msg::CountryFlag { tunnel, code } => {
                let flags = self.flags.borrow();
                if let Some(image) = flags.get(&tunnel) {
                    // prevents crash when deleting servers
                    if image.pixbuf().is_some() {
                        match util::load_embedded_pixbuf(&format!("flags/{}.png", code)) {
                            Ok(pixbuf) => image.set_from_pixbuf(Some(&pixbuf)),
                            Err(e) => eprintln!(
                                "error loading embedded image flags/{}.png: {}",
                                code, e
                            ),
                        }
                    }
                }
            }
            Msg::UpdateAvailable { tag } => {
                let dialog = gtk::MessageDialog::new(
                    Some(&self.window),
                    gtk::DialogFlags::MODAL,
                    gtk::MessageType::Info,
                    gtk::ButtonsType::Ok,
                    &format!("wg-gui: update available => {}", tag),
                );
                dialog.run();
                util::destroy(&dialog);
            }
            Msg::Log { level, text } => self.wlog(&level, &text),
            Msg::Error(text) => self.show_error(&text),
            Msg::Rescan => {
                if let Err(e) = self.scan_tunnels() {
                    self.show_error(&e);
                }
            }
        }
    }

    fn tick(&self) {
        let devices = wg::devices().unwrap_or_default();
        let active_names: Vec<String> = devices.iter().map(|d| d.name.clone()).collect();

        self.set_indicator_icon(if active_names.is_empty() {
            "wireguard_off"
        } else {
            "wg_connected"
        });

        if !self.window.is_visible() {
            return;
        }

        for (name, icon) in self.icons.borrow().iter() {
            if active_names.contains(name) {
                icon.set_from_pixbuf(Some(&self.green_icon));
            } else {
                icon.set_from_pixbuf(Some(&self.gray_icon));
            }
        }

        if active_names.is_empty() {
            self.header.set_subtitle(Some("Not connected!"));
        } else {
            self.header
                .set_subtitle(Some(&format!("Connected to {}", active_names.join(", "))));
        }

        let row = match self.list.selected_row() {
            Some(row) => row,
            None => {
                self.unknown_labels();
                return;
            }
        };

        let name = row.widget_name().to_string();

        if !active_names.contains(&name) {
            self.iface_status.set_text("Inactive");
            self.iface_public_key.set_text("unknown");
            self.iface_listen_port.set_text("unknown");
            self.button_change_state.set_label("Activate");
            self.set_peer_stats_unknown();
            return;
        }

        if let Some(device) = devices.iter().find(|d| d.name == name) {
            self.iface_status.set_text("Active");
            self.iface_public_key.set_text(&device.public_key);
            self.iface_listen_port
                .set_text(&device.listen_port.to_string());
            self.button_change_state.set_label("Deactivate");
            self.set_peer_stats(&device.peers);
        }
    }

    // updates the live stats of every displayed peer
    fn set_peer_stats(&self, peers: &[wg::Peer]) {
        let mut live: HashMap<&str, &wg::Peer> = HashMap::new();
        for peer in peers {
            live.insert(peer.public_key.as_str(), peer);
        }

        for row in self.peer_rows.borrow().iter() {
            match live.get(row.public_key.as_str()) {
                Some(peer) => {
                    row.active.set(true);
                    row.handshake.set_text(&humanize::time(peer.latest_handshake));
                    row.transfer
                        .set_text(&tunnels::peer_transfer_string(peer));
                    if let Some(endpoint) = &peer.endpoint {
                        row.endpoint.set_text(endpoint);
                    }
                }
                None => {
                    row.active.set(false);
                    row.handshake.set_text("unknown");
                    row.transfer.set_text("unknown");
                }
            }

            row.button.set_sensitive(true);
            if row.active.get() {
                row.button.set_label("Deactivate");
                row.button
                    .set_tooltip_text(Some("Remove this peer from the running interface"));
            } else {
                row.button.set_label("Activate");
                row.button
                    .set_tooltip_text(Some("Add this peer back to the running interface"));
            }
        }
    }

    // resets the live stats of every displayed peer
    fn set_peer_stats_unknown(&self) {
        for row in self.peer_rows.borrow().iter() {
            row.active.set(false);
            row.handshake.set_text("unknown");
            row.transfer.set_text("unknown");

            row.button.set_label("Activate");
            row.button.set_sensitive(false);
        }
    }

    fn unknown_labels(&self) {
        self.button_change_state.set_label("unknown");
        self.iface_addresses.set_text("unknown");
        self.iface_status.set_text("unknown");
        self.iface_dns.set_text("unknown");
        self.iface_public_key.set_text("unknown");
        self.iface_listen_port.set_text("unknown");
        self.set_peer_stats_unknown();
    }

    #[allow(clippy::wrong_self_convention)]
    fn from_settings(&self) -> Result<(), String> {
        let settings = self.settings.lock().unwrap();

        self.settings_multiple.set_active(settings.multiple_tunnels);
        self.settings_start_tray.set_active(settings.start_on_tray);
        self.settings_check_updates.set_active(settings.check_updates);
        self.settings_country_flags.set_active(settings.country_flags);
        self.settings_remember
            .set_active(settings.remember_connected_servers);
        self.settings_polkit.set_active(settings.passwordless);

        Ok(())
    }

    fn to_settings(&self) {
        let mut settings = self.settings.lock().unwrap();

        settings.multiple_tunnels = self.settings_multiple.is_active();
        settings.start_on_tray = self.settings_start_tray.is_active();
        settings.check_updates = self.settings_check_updates.is_active();
        settings.country_flags = self.settings_country_flags.is_active();
        settings.remember_connected_servers = self.settings_remember.is_active();
        settings.passwordless = self.settings_polkit.is_active();
    }

    fn set_passwordless(&self) -> Result<(), String> {
        const POLICY: &str = "/usr/share/polkit-1/actions/wg-gui.policy";

        // not installed
        if !Path::new(POLICY).exists() {
            return Ok(());
        }

        let content = std::fs::read_to_string(POLICY).map_err(|e| e.to_string())?;
        let passwordless = self.settings.lock().unwrap().passwordless;

        let updated = if passwordless {
            content
                .replace(
                    "<allow_any>auth_admin</allow_any>",
                    "<allow_any>yes</allow_any>",
                )
                .replace(
                    "<allow_inactive>auth_admin</allow_inactive>",
                    "<allow_inactive>yes</allow_inactive>",
                )
                .replace(
                    "<allow_active>auth_admin_keep</allow_active>",
                    "<allow_active>yes</allow_active>",
                )
                .replace(
                    "<allow_active>auth_admin</allow_active>",
                    "<allow_active>yes</allow_active>",
                )
        } else {
            content
                .replace(
                    "<allow_any>yes</allow_any>",
                    "<allow_any>auth_admin</allow_any>",
                )
                .replace(
                    "<allow_inactive>yes</allow_inactive>",
                    "<allow_inactive>auth_admin</allow_inactive>",
                )
                .replace(
                    "<allow_active>yes</allow_active>",
                    "<allow_active>auth_admin_keep</allow_active>",
                )
        };

        if updated != content {
            std::fs::write(POLICY, updated).map_err(|e| e.to_string())?;
        }

        Ok(())
    }

    fn set_indicator_icon(&self, name: &str) {
        let mut indicator = self.indicator.borrow_mut();
        if let Some(indicator) = indicator.as_mut() {
            indicator.set_icon(name);
        }
    }

    fn show_error(&self, error: &str) {
        if error.is_empty() {
            return;
        }

        self.wlog("ERROR", error);

        let dialog = gtk::MessageDialog::new(
            Some(&self.window),
            gtk::DialogFlags::MODAL,
            gtk::MessageType::Error,
            gtk::ButtonsType::Ok,
            error,
        );
        dialog.set_title("Error");
        dialog.run();
        util::destroy(&dialog);
    }

    fn wlog(&self, level: &str, text: &str) {
        let level = if level == "ERROR" {
            "<span color=\"#FF0000\">ERROR</span>".to_string()
        } else {
            level.to_string()
        };

        let timestamp = glib::DateTime::now_local()
            .ok()
            .and_then(|now| now.format("%d/%b/%y %H:%M:%S %Z").ok())
            .map(|s| s.to_string())
            .unwrap_or_default();

        let label = gtk::Label::new(None);
        label.set_markup(&format!(
            "<span color=\"#008080\">[{}]</span>[{}]: {}",
            timestamp, level, text
        ));
        label.set_hexpand(true);
        label.set_halign(gtk::Align::Start);
        label.show();

        self.logs.add(&label);
    }

    // resizes the window so that all peers of the selected tunnel are visible
    // without scrolling
    pub fn fit_window(&self) {
        if self.peer_scroller.allocated_height() <= 0 {
            return;
        }

        let (_, content) = self.peer_list.preferred_height();
        let extra = content - self.peer_scroller.allocated_height();
        if extra == 0 {
            return;
        }

        let (width, height) = self.window.size();
        let mut wanted = height + extra;

        // don't grow beyond the monitor's work area
        let mut max_height = 0;
        if let Some(display) = gtk::gdk::Display::default() {
            let monitor = display.primary_monitor().or_else(|| {
                if display.n_monitors() > 0 {
                    display.monitor(0)
                } else {
                    None
                }
            });

            if let Some(monitor) = monitor {
                max_height = monitor.workarea().height();
            }
        }

        if max_height > 0 {
            wanted = wanted.min(max_height);
        }

        if wanted > 0 {
            self.window.resize(width, wanted);
        }
    }

    fn schedule_fit_window(self: &Rc<Self>) {
        let app = self.clone();
        glib::idle_add_local_once(move || app.fit_window());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // GTK may only be used from the thread that initialized it, so every
    // widget-touching assertion lives in a single test that runs on one thread.
    #[test]
    fn gtk_ui_smoke() {
        if gtk::init().is_err() {
            eprintln!("skipping gtk ui smoke test: no display");
            return;
        }

        glade_contains_every_widget();
        loads_embedded_assets();
        builds_the_whole_ui_without_root();
        indicator_library_loads();
    }

    fn glade_contains_every_widget() {
        let glade = util::asset_string("wg-gui.glade").expect("glade asset");
        let builder = gtk::Builder::from_string(&glade);

        let ids = [
            "main_window",
            "main_header",
            "label_version",
            "button_settings",
            "button_quit",
            "tunnel_list",
            "box_peers",
            "label_interface_status",
            "label_interface_public_key",
            "label_interface_listen_port",
            "label_interface_addresses",
            "label_interface_dns_servers",
            "button_change_state",
            "button_add_tunnel",
            "button_del_tunnel",
            "button_zip_tunnel",
            "button_edit_tunnel",
            "wireguard_logs",
            "editor_window",
            "editor_name",
            "editor_publickey",
            "editor_text",
            "editor_button_save",
            "editor_button_cancel",
            "settings_window",
            "settings_multiple_active_tunnels",
            "settings_start_tray",
            "settings_check_updates",
            "settings_country_flags",
            "settings_remember_connected",
            "settings_polkit_policy",
            "button_settings_save",
            "button_settings_cancel",
            "icon_delete",
            "icon_tunnel",
            "icon_zip",
        ];

        for id in ids {
            assert!(
                builder.object::<glib::Object>(id).is_some(),
                "missing widget: {}",
                id
            );
        }
    }

    fn loads_embedded_assets() {
        assert!(util::asset("flags/IDK.png").is_some());
        assert!(util::asset("flags/US.png").is_some());
        assert!(util::asset("Icon/connected.png").is_some());
        assert!(util::asset("Icon/not_connected.png").is_some());
        assert!(util::load_embedded_pixbuf("Icon/connected.png").is_ok());
    }

    // the UI constructor must not need root; wg and /etc/wireguard are only
    // touched once the window setup continues with scan_tunnels()
    fn builds_the_whole_ui_without_root() {
        let glade = util::asset_string("wg-gui.glade").expect("glade asset");
        let builder = gtk::Builder::from_string(&glade);
        let window = builder
            .object::<gtk::ApplicationWindow>("main_window")
            .expect("main window");

        let application = gtk::Application::new(None, gtk::gio::ApplicationFlags::empty());
        application.add_window(&window);

        #[allow(deprecated)]
        let (tx, _rx) = glib::MainContext::channel::<Msg>(glib::Priority::DEFAULT);

        let settings = Arc::new(Mutex::new(Settings::default()));
        let tray_menu = gtk::Menu::new();

        let fake_bin = fake_wg_bin();
        let old_path = std::env::var("PATH").unwrap_or_default();
        std::env::set_var("PATH", format!("{}:{}", fake_bin.display(), old_path));

        let app = App::create(
            &application,
            &builder,
            &window,
            None,
            tray_menu,
            settings,
            tx,
        );

        std::env::set_var("PATH", old_path);

        let app = app.expect("app create");

        // the peer scroller replaced the static single-peer frame
        assert!(app.peer_scroller.parent().is_some());
        assert_eq!(app.menu_tunnels.label().as_deref(), Some("Tunnels"));
    }

    fn fake_wg_bin() -> std::path::PathBuf {
        let dir = std::env::temp_dir().join("wg-gui-test-bin");
        std::fs::create_dir_all(&dir).expect("create fake bin dir");

        let path = dir.join("wg");
        std::fs::write(
            &path,
            "#!/bin/sh\nif [ \"$1\" = show ]; then printf 'wg0\\tpriv\\tpub\\t51820\\toff\\n'; fi\nexit 0\n",
        )
        .expect("write fake wg");

        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755))
            .expect("chmod fake wg");

        dir
    }

    fn indicator_library_loads() {
        // dlopens libayatana-appindicator3; panics if unavailable
        let mut indicator = libappindicator::AppIndicator::new("wg-gui-test", "wireguard_off");
        indicator.set_status(libappindicator::AppIndicatorStatus::Active);
    }

    #[test]
    fn colorizes_conf_like_the_editor() {
        let out = colorize_conf("[Interface]\nPrivateKey = abc&def\n[Peer]\n");

        assert!(out.contains("<span color=\"green\">[Interface]</span>"));
        assert!(out.contains("<span color=\"green\">[Peer]</span>"));
        assert!(out.contains("<span color=\"purple\">PrivateKey</span>"));
        assert!(out.contains("abc&amp;def"));
    }

    #[test]
    fn finds_first_public_key() {
        let conf = "# comment\n[Interface]\nAddress = 10.0.0.1\n[Peer]\nPublicKey = key1\n[Peer]\nPublicKey = key2\n";
        assert_eq!(find_public_key(conf).as_deref(), Some("key1"));
        assert_eq!(find_public_key("[Interface]\nAddress = 10.0.0.1\n"), None);
    }
}
