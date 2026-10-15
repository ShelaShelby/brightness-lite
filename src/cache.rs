//! In-memory copy of what the monitors showed at the last scan or write.
//! The window draws from this only; it never reads a monitor.

use crate::{ddc::Setting, store};

type Rect = (i32, i32, i32, i32);

/// One monitor as the window sees it.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct View {
    /// Windows display number, if known.
    pub display_number: Option<u32>,
    /// Stable id of the physical monitor, if known.
    pub key: Option<String>,
    /// Position on the desktop, if known.
    pub rect: Option<Rect>,
    /// Brightness and contrast as 0-100, None if the monitor never answered.
    pub values: [Option<u32>; 2],
    /// Messages about settings that failed on this monitor.
    pub errors: Vec<String>,
}

impl View {
    pub fn label(&self) -> String {
        crate::ddc::label_for(self.display_number)
    }

    pub fn value(&self, st: Setting) -> Option<u32> {
        self.values[st.index()]
    }

    pub fn set_value(&mut self, st: Setting, v: u32) {
        self.values[st.index()] = Some(crate::ddc::clamp(v as i64));
    }
}

#[derive(Default)]
pub struct Cache {
    pub views: Vec<View>,
    /// False until the first scan has finished.
    pub scanned: bool,
    /// Counts scans, so a write meant for an old monitor list can be dropped.
    pub generation: u64,
}

impl Cache {
    pub fn replace(&mut self, views: Vec<View>, generation: u64) {
        self.views = views;
        self.generation = generation;
        self.scanned = true;
    }

    /// Error messages of all monitors, in order.
    pub fn error_lines(&self) -> Vec<String> {
        self.views.iter().flat_map(|v| v.errors.clone()).collect()
    }
}

/// Settings to write to disk, built from cached values alone (no monitor read).
pub fn to_settings(views: &[View], old: &store::Settings) -> store::Settings {
    let current: Vec<store::Current> = views
        .iter()
        .filter_map(|v| {
            Some(store::Current {
                key: v.key.as_deref()?,
                brightness: v.value(Setting::Brightness),
                contrast: v.value(Setting::Contrast),
            })
        })
        .collect();
    store::merged(old, &current)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn view(key: &str, b: Option<u32>, c: Option<u32>) -> View {
        View { key: Some(key.into()), display_number: Some(1), values: [b, c], ..Default::default() }
    }

    #[test]
    fn cache_returns_last_values_without_a_monitor_read() {
        let mut cache = Cache::default();
        cache.replace(vec![view("a", Some(40), Some(60))], 1);
        // A slider moved: the cache takes the new value straight away.
        cache.views[0].set_value(Setting::Brightness, 75);
        assert_eq!(cache.views[0].value(Setting::Brightness), Some(75));
        assert_eq!(cache.views[0].value(Setting::Contrast), Some(60));
        // What gets saved comes from the cache, with no DDC/CI involved.
        let saved = to_settings(&cache.views, &store::Settings::default());
        let m = saved.find("a").unwrap();
        assert_eq!((m.brightness, m.contrast), (75, 60));
    }

    #[test]
    fn unreadable_value_keeps_old_saved_number() {
        let old = store::Settings { monitors: vec![store::SavedMonitor { key: "a".into(), brightness: 30, contrast: 20 }], ..Default::default() };
        let cache = Cache { views: vec![view("a", None, Some(55))], scanned: true, generation: 1 };
        let saved = to_settings(&cache.views, &old);
        let m = saved.find("a").unwrap();
        assert_eq!((m.brightness, m.contrast), (30, 55));
    }

    #[test]
    fn set_value_is_clamped() {
        let mut v = view("a", Some(1), Some(1));
        v.set_value(Setting::Contrast, 900);
        assert_eq!(v.value(Setting::Contrast), Some(100));
    }
}
