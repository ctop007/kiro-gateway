//! Keep AppKit from offering to restore windows after an unclean exit (macOS).
//!
//! When the app ends by a signal (`kill`, logout, the updater restarting it) AppKit records a
//! "crash" and, on the next launch, shows a modal "reopen windows?" alert from inside
//! `finishLaunching`. That alert blocks the main thread before our setup runs, so the gateway
//! is never restored and the tray never appears until someone clicks it. The app has one
//! fixed window and nothing to restore, so persistence is switched off for this process.
//!
//! `ApplePersistenceIgnoreState` is read from user defaults, not from Info.plist, so it is
//! registered in the volatile registration domain: it applies to this run only, is never
//! written to disk, and an explicit user setting would still win.

use objc2::runtime::AnyObject;
use objc2_foundation::{NSDictionary, NSNumber, NSString, NSUserDefaults};

/// User-defaults key AppKit reads to skip window-state restoration.
pub const PERSISTENCE_IGNORE_STATE_KEY: &str = "ApplePersistenceIgnoreState";

/// Register `ApplePersistenceIgnoreState = YES` for this process. Must run before
/// `NSApplication` finishes launching, i.e. before the Tauri builder runs.
pub fn disable_window_restoration() {
    let key = NSString::from_str(PERSISTENCE_IGNORE_STATE_KEY);
    let value = NSNumber::new_bool(true);
    let value: &AnyObject = &value;
    let defaults = NSDictionary::<NSString, AnyObject>::from_slices(&[&*key], &[value]);
    // SAFETY: the dictionary holds an NSString key and an NSNumber value, both property-list
    // types as registerDefaults: requires.
    unsafe { NSUserDefaults::standardUserDefaults().registerDefaults(&defaults) };
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_flag_is_visible_through_user_defaults_after_registration() {
        disable_window_restoration();
        let key = NSString::from_str(PERSISTENCE_IGNORE_STATE_KEY);
        assert!(NSUserDefaults::standardUserDefaults().boolForKey(&key));
    }
}
