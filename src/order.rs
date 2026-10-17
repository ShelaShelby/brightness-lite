//! The order of the monitor cards in the window. Pure functions, no I/O.
//!
//! The saved order is a list of monitor keys. It only decides where a card is
//! shown. Values, sliders and DDC/CI writes always stay bound to the monitor
//! itself, never to a position on screen.

/// Make a saved order safe to use: drop empty keys and keep only the first
/// copy of a key that is listed twice.
pub fn clean(order: impl IntoIterator<Item = String>) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for k in order {
        if !k.trim().is_empty() && !out.contains(&k) {
            out.push(k);
        }
    }
    out
}

/// Give every monitor its own key. A key that is already taken by an earlier
/// monitor gets "#2", "#3" ... added, in the order the monitors were found.
/// Monitors without a key keep none.
pub fn make_unique(keys: &mut [Option<String>]) {
    let mut seen: Vec<String> = Vec::new();
    for slot in keys.iter_mut() {
        let Some(k) = slot.take() else { continue };
        let mut candidate = k.clone();
        let mut n = 2;
        while seen.contains(&candidate) {
            candidate = format!("{k}#{n}");
            n += 1;
        }
        seen.push(candidate.clone());
        *slot = Some(candidate);
    }
}

/// Where a monitor goes: 0 = in the saved order, 1 = has a key but is new,
/// 2 = has no key (it can never be saved, so it is always last).
fn rank(key: Option<&str>, order: &[String]) -> (u8, usize) {
    match key {
        Some(k) => match order.iter().position(|o| o == k) {
            Some(p) => (0, p),
            None => (1, 0),
        },
        None => (2, 0),
    }
}

/// Indices into `keys` (one entry per connected monitor) in the order the
/// cards are shown. Saved monitors first, in their saved order. Then new
/// monitors, then monitors without a key, both in the order they were found.
pub fn shown(keys: &[Option<&str>], order: &[String]) -> Vec<usize> {
    let mut idx: Vec<usize> = (0..keys.len()).collect();
    idx.sort_by_key(|&i| rank(keys[i], order)); // stable: ties keep the found order
    idx
}

/// True if the cards at `pos` and `pos + 1` can be swapped: both exist and
/// both monitors have a key (so the new order can be saved).
pub fn can_swap(keys: &[Option<&str>], order: &[String], pos: usize) -> bool {
    let s = shown(keys, order);
    pos + 1 < s.len() && keys[s[pos]].is_some() && keys[s[pos + 1]].is_some()
}

/// The saved order after swapping the cards shown at `pos` and `pos + 1`.
/// None if they cannot be swapped. Keys of monitors that are not connected
/// now keep their place, so an unplugged monitor comes back where it was.
pub fn swapped(keys: &[Option<&str>], order: &[String], pos: usize) -> Option<Vec<String>> {
    if !can_swap(keys, order, pos) {
        return None;
    }
    let mut s = shown(keys, order);
    s.swap(pos, pos + 1);
    let now: Vec<&str> = s.iter().filter_map(|&i| keys[i]).collect();
    // New monitors get a slot at the end, in the order they are shown.
    let mut out = clean(order.iter().cloned());
    for k in &now {
        if !out.iter().any(|o| o == k) {
            out.push(k.to_string());
        }
    }
    // Refill the slots of the connected monitors in the new on-screen order.
    let mut next = now.iter();
    for slot in out.iter_mut() {
        if now.contains(&slot.as_str()) {
            if let Some(k) = next.next() {
                *slot = k.to_string();
            }
        }
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(v: &[&str]) -> Vec<String> {
        v.iter().map(|x| x.to_string()).collect()
    }

    fn k<'a>(v: &[&'a str]) -> Vec<Option<&'a str>> {
        v.iter().map(|x| if x.is_empty() { None } else { Some(*x) }).collect()
    }

    /// The keys in the order they are shown ("" = no key).
    fn names<'a>(keys: &[Option<&'a str>], order: &[String]) -> Vec<&'a str> {
        shown(keys, order).iter().map(|&i| keys[i].unwrap_or("")).collect()
    }

    #[test]
    fn one_monitor_cannot_swap() {
        let keys = k(&["a"]);
        assert_eq!(shown(&keys, &[]), vec![0]);
        assert!(!can_swap(&keys, &[], 0));
        assert_eq!(swapped(&keys, &[], 0), None);
        assert_eq!(swapped(&[], &[], 0), None);
    }

    #[test]
    fn two_monitors_swap_and_swap_back() {
        let keys = k(&["a", "b"]);
        let o1 = swapped(&keys, &[], 0).unwrap();
        assert_eq!(names(&keys, &o1), vec!["b", "a"]);
        let o2 = swapped(&keys, &o1, 0).unwrap();
        assert_eq!(names(&keys, &o2), vec!["a", "b"]);
        assert_eq!(swapped(&keys, &o2, 1), None); // no card below the last one
    }

    #[test]
    fn three_monitors_first_middle_last() {
        let keys = k(&["a", "b", "c"]);
        let o = swapped(&keys, &[], 0).unwrap();
        assert_eq!(names(&keys, &o), vec!["b", "a", "c"]);
        let o = swapped(&keys, &o, 1).unwrap();
        assert_eq!(names(&keys, &o), vec!["b", "c", "a"]);
        let o = swapped(&keys, &o, 0).unwrap();
        assert_eq!(names(&keys, &o), vec!["c", "b", "a"]);
    }

    #[test]
    fn swap_twice_returns_to_the_start_everywhere() {
        let keys = k(&["a", "b", "c", "d", "e"]);
        let start = s(&["c", "gone", "a", "e"]);
        for pos in 0..4 {
            let once = swapped(&keys, &start, pos).unwrap();
            let twice = swapped(&keys, &once, pos).unwrap();
            assert_eq!(names(&keys, &twice), names(&keys, &start), "pos {pos}");
        }
    }

    /// Every swap of every pair moves exactly those two cards, for many layouts.
    #[test]
    fn swap_moves_exactly_the_two_cards() {
        let all = ["a", "b", "c", "d"];
        let orders = [s(&[]), s(&["d", "a"]), s(&["x", "c", "b", "a", "d"]), s(&["b", "", "b", "zz"])];
        for n in 1..=all.len() {
            let mut keys = k(&all[..n]);
            for with_keyless in [false, true] {
                if with_keyless {
                    keys.insert(0, None);
                }
                for order in &orders {
                    let before = names(&keys, order);
                    for pos in 0..keys.len() {
                        match swapped(&keys, order, pos) {
                            Some(next) => {
                                let mut want = before.clone();
                                want.swap(pos, pos + 1);
                                assert_eq!(names(&keys, &next), want, "keys {keys:?} order {order:?} pos {pos}");
                                assert_eq!(clean(next.clone()), next, "result has no junk");
                            }
                            None => {
                                assert!(pos + 1 >= keys.len() || before[pos].is_empty() || before[pos + 1].is_empty())
                            }
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn saved_monitor_that_is_not_connected_keeps_its_place() {
        let order = s(&["a", "b", "c"]);
        // "a" unplugged: b and c are swapped.
        let keys = k(&["b", "c"]);
        assert_eq!(names(&keys, &order), vec!["b", "c"]);
        let o = swapped(&keys, &order, 0).unwrap();
        assert_eq!(o, s(&["a", "c", "b"]));
        // "a" plugged in again, found in another position: back on top.
        let keys = k(&["c", "a", "b"]);
        assert_eq!(names(&keys, &o), vec!["a", "c", "b"]);
    }

    #[test]
    fn new_monitor_goes_to_the_end_and_can_be_moved() {
        let order = s(&["b", "a"]);
        let keys = k(&["new", "a", "b"]);
        assert_eq!(names(&keys, &order), vec!["b", "a", "new"]);
        // Swapping the new one up must work (it is not in the saved order yet).
        let o = swapped(&keys, &order, 1).unwrap();
        assert_eq!(names(&keys, &o), vec!["b", "new", "a"]);
        assert_eq!(o, s(&["b", "new", "a"]));
    }

    #[test]
    fn monitors_without_key_are_last_and_not_swappable() {
        let keys = k(&["", "a", "b"]);
        assert_eq!(names(&keys, &[]), vec!["a", "b", ""]);
        assert!(can_swap(&keys, &[], 0));
        assert!(!can_swap(&keys, &[], 1));
        assert_eq!(swapped(&keys, &[], 1), None);
    }

    #[test]
    fn junk_in_the_saved_order_is_ignored() {
        let keys = k(&["a", "b"]);
        assert_eq!(names(&keys, &s(&["", "  ", "b", "b", "a", "b"])), vec!["b", "a"]);
        assert_eq!(clean(s(&["", "b", " ", "b", "a", "b"])), s(&["b", "a"]));
        let o = swapped(&keys, &s(&["", "b", "b", "a"]), 0).unwrap();
        assert_eq!(o, s(&["a", "b"]));
    }

    #[test]
    fn missing_order_uses_the_found_order() {
        assert_eq!(names(&k(&["c", "a", "b"]), &[]), vec!["c", "a", "b"]);
    }

    #[test]
    fn identical_monitors_get_unique_keys() {
        let mut keys = vec![Some("m".to_string()), None, Some("m".into()), Some("x".into()), Some("m".into())];
        make_unique(&mut keys);
        assert_eq!(keys, vec![Some("m".into()), None, Some("m#2".into()), Some("x".into()), Some("m#3".into())]);
        // A real key that already looks like a made-up one is not doubled.
        let mut keys = vec![Some("m#2".to_string()), Some("m".into()), Some("m".into())];
        make_unique(&mut keys);
        assert_eq!(keys, vec![Some("m#2".into()), Some("m".into()), Some("m#3".into())]);
    }
}
