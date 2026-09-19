//! Read-only bridge from GNOME and portal color-scheme preferences to GTK.

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use gio::prelude::*;
use glib::variant::ToVariant;

/// Retains live system-preference subscriptions for the application lifetime.
pub struct SystemThemeBridge {
    _desktop_settings: Option<gio::Settings>,
    _portal: Rc<RefCell<Option<gio::DBusProxy>>>,
}

/// Follows GNOME and sandbox portal preferences without writing host settings.
pub fn inherit_system_color_scheme() -> SystemThemeBridge {
    let portal = Rc::new(RefCell::new(None));
    let desktop_settings = inherit_gnome_color_scheme();
    if std::env::var_os("FLATPAK_ID").is_some() || std::env::var_os("APPIMAGE").is_some() {
        subscribe_portal_color_scheme(&portal);
    }
    SystemThemeBridge { _desktop_settings: desktop_settings, _portal: portal }
}

fn inherit_gnome_color_scheme() -> Option<gio::Settings> {
    let schema_source = gio::SettingsSchemaSource::default()?;
    let schema = schema_source.lookup("org.gnome.desktop.interface", true)?;
    if !schema.has_key("color-scheme") { return None; }
    let settings = gio::Settings::new_full(&schema, None::<&gio::SettingsBackend>, None::<&str>);
    let fallback = gtk::Settings::default().is_some_and(|s| s.is_gtk_application_prefer_dark_theme());
    apply_gnome(&settings, fallback);
    settings.connect_changed(Some("color-scheme"), move |settings, _| apply_gnome(settings, fallback));
    Some(settings)
}

fn subscribe_portal_color_scheme(storage: &Rc<RefCell<Option<gio::DBusProxy>>>) {
    let weak = Rc::downgrade(storage);
    let fallback = gtk::Settings::default().is_some_and(|s| s.is_gtk_application_prefer_dark_theme());
    glib::MainContext::default().spawn_local(async move {
        let Ok(proxy) = gio::DBusProxy::for_bus_future(gio::BusType::Session, gio::DBusProxyFlags::DO_NOT_LOAD_PROPERTIES, None, "org.freedesktop.portal.Desktop", "/org/freedesktop/portal/desktop", "org.freedesktop.portal.Settings").await else { return; };
        let changed = Arc::new(AtomicBool::new(false));
        let signal_changed = Arc::clone(&changed);
        proxy.connect_g_signal(move |_, _, signal, parameters| {
            let Some((namespace, key, value)) = parameters.get::<(String, String, glib::Variant)>() else { return; };
            if signal == "SettingChanged" && namespace == "org.freedesktop.appearance" && key == "color-scheme"
                && let Some(dark) = portal_prefer_dark(&value, fallback) {
                signal_changed.store(true, Ordering::Relaxed);
                if let Some(settings) = gtk::Settings::default() { settings.set_gtk_application_prefer_dark_theme(dark); }
            }
        });
        let reply = proxy.call_future("ReadOne", Some(&("org.freedesktop.appearance", "color-scheme").to_variant()), gio::DBusCallFlags::NONE, 1500).await;
        let Some(storage) = weak.upgrade() else { return; };
        if !changed.load(Ordering::Relaxed)
            && let Ok(reply) = reply
            && let Some((value,)) = reply.get::<(glib::Variant,)>()
            && let Some(dark) = portal_prefer_dark(&value, fallback)
            && let Some(settings) = gtk::Settings::default() { settings.set_gtk_application_prefer_dark_theme(dark); }
        *storage.borrow_mut() = Some(proxy);
    });
}

fn portal_prefer_dark(value: &glib::Variant, fallback: bool) -> Option<bool> {
    value.get::<u32>().map(|value| match value { 1 => true, 2 => false, _ => fallback })
}

fn apply_gnome(settings: &gio::Settings, fallback: bool) {
    let Some(gtk_settings) = gtk::Settings::default() else { return; };
    let dark = match settings.string("color-scheme").as_str() { "prefer-dark" => true, "prefer-light" => false, _ => fallback };
    gtk_settings.set_gtk_application_prefer_dark_theme(dark);
}
