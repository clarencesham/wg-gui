mod conf;
mod countries;
mod endpoints;
mod gui;
mod humanize;
mod settings;
mod tunnels;
mod util;
mod wg;

use std::path::Path;
use std::sync::{Arc, Mutex, OnceLock};

use gtk::glib;
use gtk::prelude::*;

use settings::Settings;

static LOCK_FILE: OnceLock<std::fs::File> = OnceLock::new();

fn main() {
    if unsafe { libc::geteuid() } != 0 {
        print_root_banner();
        return;
    }

    // gtk.Application's own single-instance handling relies on the session bus,
    // which is unavailable when running as root via pkexec, so a lock file is
    // used to stop instances from piling up.
    if !acquire_single_instance() {
        return;
    }

    let settings = Arc::new(Mutex::new(Settings::load()));

    #[allow(deprecated)]
    let (tx, rx) = glib::MainContext::channel::<gui::Msg>(glib::Priority::DEFAULT);

    // another launch (e.g. clicking the desktop icon again) asks the running
    // instance to bring its window back instead of starting a second process
    match signal_hook::iterator::Signals::new([signal_hook::consts::signal::SIGUSR1]) {
        Ok(mut signals) => {
            let signal_tx = tx.clone();
            std::thread::spawn(move || {
                for _ in signals.forever() {
                    let _ = signal_tx.send(gui::Msg::ShowWindow);
                }
            });
        }
        Err(e) => eprintln!("error installing signal handler: {}", e),
    }

    let application = gtk::Application::new(
        Some("com.wireguard.desktop"),
        gtk::gio::ApplicationFlags::empty(),
    );

    let rx_cell = std::cell::RefCell::new(Some(rx));
    let settings_for_window = settings.clone();
    let tx_for_window = tx.clone();

    application.connect_activate(move |app| {
        let rx = match rx_cell.borrow_mut().take() {
            Some(rx) => rx,
            None => return,
        };

        if let Err(e) = create_window(app, settings_for_window.clone(), tx_for_window.clone(), rx) {
            eprintln!("create window error: {}", e);
        }
    });

    let exit_code = application.run();
    std::process::exit(i32::from(exit_code));
}

// acquire_single_instance takes an exclusive lock for the process lifetime and
// returns whether this process should run. If another instance holds the lock,
// it is asked to show its window and this process exits.
fn acquire_single_instance() -> bool {
    let mut lock_path = std::path::PathBuf::from("/run/wg-gui.lock");

    let mut file = match std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(&lock_path)
    {
        Ok(file) => file,
        Err(_) => {
            lock_path = std::env::temp_dir().join("wg-gui.lock");

            match std::fs::OpenOptions::new()
                .read(true)
                .write(true)
                .create(true)
                .truncate(false)
                .open(&lock_path)
            {
                Ok(file) => file,
                Err(e) => {
                    eprintln!("error opening single instance lock file: {}", e);
                    return true;
                }
            }
        }
    };

    match file.try_lock() {
        Ok(()) => {
            use std::io::{Seek, SeekFrom, Write};

            // record our pid so the next launch can ask us to show the window
            let _ = file.set_len(0);
            let _ = file.seek(SeekFrom::Start(0));
            let _ = writeln!(file, "{}", std::process::id());
            let _ = file.sync_all();

            // keep the file open so the lock is held until the process exits
            let _ = LOCK_FILE.set(file);

            true
        }
        Err(std::fs::TryLockError::WouldBlock) => {
            if let Ok(data) = std::fs::read_to_string(&lock_path) {
                if let Ok(pid) = data.trim().parse::<i32>() {
                    if pid > 0 {
                        unsafe {
                            libc::kill(pid, libc::SIGUSR1);
                        }
                    }
                }
            }

            println!("wg-gui is already running, showing its window.");
            false
        }
        Err(std::fs::TryLockError::Error(_)) => true,
    }
}

fn create_tray(
    application: &gtk::Application,
    window: &gtk::ApplicationWindow,
) -> Result<(Option<libappindicator::AppIndicator>, gtk::Menu), String> {
    let mut menu = gtk::Menu::new();

    let menu_show = gtk::MenuItem::with_label("Show");
    let menu_quit = gtk::MenuItem::with_label("Quit");

    {
        let window = window.clone();
        menu_show.connect_activate(move |_| {
            window.show();
            window.present();
        });
    }

    {
        let application = application.clone();
        menu_quit.connect_activate(move |_| application.quit());
    }

    menu.append(&menu_show);
    menu.append(&menu_quit);

    let icons_dir = tray_icons_dir()?;

    let indicator = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let mut indicator =
            libappindicator::AppIndicator::new("com.wireguard.desktop", "wireguard_off");
        indicator.set_icon_theme_path(&icons_dir);
        indicator.set_title("wg-gui");
        indicator.set_status(libappindicator::AppIndicatorStatus::Active);
        indicator.set_menu(&mut menu);
        indicator
    }));

    let indicator = match indicator {
        Ok(indicator) => Some(indicator),
        Err(_) => {
            eprintln!("create tray error");
            None
        }
    };

    menu.show_all();

    Ok((indicator, menu))
}

fn tray_icons_dir() -> Result<String, String> {
    let installed = "/usr/share/wg-gui/Icon/";
    if Path::new(installed).exists() {
        return Ok(installed.to_string());
    }

    let tmp_dir = std::env::temp_dir();
    for icon in ["wireguard_off", "wg_connected"] {
        let data = util::asset(&format!("Icon/{}.png", icon))
            .ok_or_else(|| format!("missing embedded icon: {}", icon))?;

        std::fs::write(tmp_dir.join(format!("{}.png", icon)), data).map_err(|e| e.to_string())?;
    }

    Ok(tmp_dir.to_string_lossy().into_owned())
}

fn create_window(
    application: &gtk::Application,
    settings: Arc<Mutex<Settings>>,
    tx: glib::Sender<gui::Msg>,
    rx: glib::Receiver<gui::Msg>,
) -> Result<(), String> {
    let glade =
        util::asset_string("wg-gui.glade").ok_or_else(|| "cant read wg-gui.glade".to_string())?;
    let builder = gtk::Builder::from_string(&glade);

    let window = builder
        .object::<gtk::ApplicationWindow>("main_window")
        .ok_or_else(|| "error getting main_window".to_string())?;

    application.add_window(&window);

    {
        let application = application.clone();
        window.connect_destroy(move |_| application.quit());
    }

    window.set_title("wg-gui");

    let (indicator, tray_menu) = create_tray(application, &window)?;

    let app = gui::App::create(
        application,
        &builder,
        &window,
        indicator,
        tray_menu,
        settings.clone(),
        tx,
    )?;

    app.scan_tunnels()?;

    {
        let app = app.clone();
        rx.attach(None, move |msg| {
            app.handle_msg(msg);
            glib::ControlFlow::Continue
        });
    }

    // once the window has been laid out, fit it to the space the peer list
    // needs; the default size already fits a single peer
    {
        let fitted = std::rc::Rc::new(std::cell::Cell::new(false));
        let app = app.clone();
        window.connect_size_allocate(move |_, _| {
            if fitted.get() {
                return;
            }

            fitted.set(true);
            app.fit_window();
        });
    }

    if !settings.lock().unwrap().start_on_tray {
        window.show_all();
    }

    Ok(())
}

fn print_root_banner() {
    println!("\n\n\n\n");
    println!("===========================================================================");
    println!("===========================================================================");
    println!("===========================================================================");
    println!("===                                                                     ===");
    println!("===                                                                     ===");
    println!("=== wg-gui must be ran as root to access /etc/wireguard and wg-quick ===");
    println!("=== Use pkexec to run as root:                                          ===");
    println!("===  pkexec wg-gui                                                   ===");
    println!("===                                                                     ===");
    println!("===                                                                     ===");
    println!("===========================================================================");
    println!("===========================================================================");
    println!("===========================================================================");
}
