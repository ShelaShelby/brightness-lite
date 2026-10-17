//! In-memory copy of what the monitors showed at the last scan or write.
//! The window draws from this only; it never reads a monitor.

use crate::{ddc::Setting, store};

type Rect = (i32, i32, i32, i32);

/// One monitor as the window sees it.
#[derive(Clone, Debug, PartialEq)]
pub struct View {
    /// Display number ("Monitor 2"), if known.
    pub display_number: Option<u32>,
    /// Model name from the monitor's EDID, if known.
    pub name: Option<String>,
    /// Stable id of the physical monitor, if known.
    pub key: Option<String>,
    /// Position on the desktop, if known.
    pub rect: Option<Rect>,
    /// Brightness and contrast as 0-100, None if the monitor never answered.
    pub values: [Option<u32>; 2],
    /// False for a setting the monitor said it does not support (its slider is hidden).
    pub supported: [bool; 2],
    /// Messages about settings that failed on this monitor.
    pub errors: Vec<String>,
}

impl Default for View {
    fn default() -> View {
        View {
            display_number: None,
            name: None,
            key: None,
            rect: None,
            values: [None; 2],
            supported: [true; 2],
            errors: Vec::new(),
        }
    }
}

impl View {
    pub fn label(&self) -> String {
        crate::ddc::label_for(self.display_number)
    }

    pub fn supports(&self, st: Setting) -> bool {
        self.supported[st.index()]
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
    /// Counts changes of the monitor list, so a write meant for an old list can be dropped.
    pub generation: u64,
    /// Hints from the last scan (why nothing was found, and so on).
    pub notes: Vec<String>,
}

impl Cache {
    pub fn replace(&mut self, views: Vec<View>, generation: u64, notes: Vec<String>) {
        self.views = views;
        self.generation = generation;
        self.notes = notes;
        self.scanned = true;
    }

    /// Error messages of all monitors, in order.
    pub fn error_lines(&self) -> Vec<String> {
        self.views.iter().flat_map(|v| v.errors.clone()).collect()
    }
}

/// Indices into `views` in the order the cards are shown (see `order::shown`).
pub fn display_order(views: &[View], order: &[String]) -> Vec<usize> {
    crate::order::shown(&keys(views), order)
}

/// The key of each view, in list order.
pub fn keys(views: &[View]) -> Vec<Option<&str>> {
    views.iter().map(|v| v.key.as_deref()).collect()
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
        cache.replace(vec![view("a", Some(40), Some(60))], 1, Vec::new());
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
        let old = store::Settings {
            monitors: vec![store::SavedMonitor { key: "a".into(), brightness: 30, contrast: 20 }],
            ..Default::default()
        };
        let cache =
            Cache { views: vec![view("a", None, Some(55))], scanned: true, generation: 1, ..Default::default() };
        let saved = to_settings(&cache.views, &old);
        let m = saved.find("a").unwrap();
        assert_eq!((m.brightness, m.contrast), (30, 55));
    }

    fn keyed(key: Option<&str>) -> View {
        View { key: key.map(Into::into), ..Default::default() }
    }

    #[test]
    fn display_order_uses_the_view_keys() {
        let views = [keyed(Some("new")), keyed(None), keyed(Some("b")), keyed(Some("a"))];
        let order = vec!["gone".to_string(), "a".into(), "b".into()];
        assert_eq!(display_order(&views, &order), vec![3, 2, 0, 1]);
    }

    #[test]
    fn set_value_is_clamped() {
        let mut v = view("a", Some(1), Some(1));
        v.set_value(Setting::Contrast, 900);
        assert_eq!(v.value(Setting::Contrast), Some(100));
    }
}
