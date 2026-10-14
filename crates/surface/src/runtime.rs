//! The live engine: input events in, [`Effect`]s out.
//!
//! Portions © RAD Films - Roman Alurkoff (from TactileBridge), MIT: dial focus by key (the
//! "adjustment dial toggle"), tool-following pages, the coalescing flush, echo suppression and
//! Auto Sync's idle timeout come from TactileBridge (`ToolContextFolder`, `CaptureOneAdjustment`,
//! `TipcBridgeService`).
//!
//! Time is passed in (milliseconds on any monotonic clock), so everything here is deterministic
//! and testable. The host:
//! 1. calls [`Runtime::set_context`] when what the app shows changes;
//! 2. feeds surface input to [`Runtime::input`];
//! 3. calls [`Runtime::flush`] every frame (and [`Runtime::interrupt`] when the active photo
//!    changes or the app takes over the edit);
//! 4. runs the effects: `Begin`/`End` bracket one undoable gesture, `Adjust` moves a value by a
//!    delta, `Command` runs a command by id.

use std::collections::BTreeMap;

use serde_json::Value;

use crate::binding::{Binding, Modifier};
use crate::devices::{PageGroup, group_of, is_encoder};
use crate::motion::{Accelerator, delta, modifier_factor};
use crate::profile::{Page, Profile};
use crate::settings::SurfaceSettings;
use crate::targets::target_spec;

/// Input from a surface.
#[derive(Clone, Debug, PartialEq)]
pub enum Input {
    /// An encoder turned by `ticks` detents (positive = clockwise).
    Turn { control: String, ticks: i32 },
    /// A key or button went down. `page` (1-based) is the key page the device itself shows, for
    /// devices that page their keys in hardware (the MX Creative Keypad's page buttons).
    Press { control: String, page: Option<usize> },
    /// A key or button came up.
    Release { control: String },
}

/// What the host should do.
#[derive(Clone, Debug, PartialEq)]
pub enum Effect {
    /// Run a command by id (engine or UI command).
    Command { command: String, params: Value },
    /// A dial gesture starts on `target`: open one undo step (develop and mask values).
    Begin { target: String },
    /// Move `target` by `delta` (in the target's units, already clamped to its range span).
    Adjust { target: String, delta: f64 },
    /// The gesture on `target` ends: commit its undo step; with `apply_to_selected`, the change
    /// goes to every selected photo in that same step.
    End { target: String, apply_to_selected: bool },
    /// Put `target` back to its default (its own undo step).
    Reset { target: String },
    /// The surface's own state changed (page, profile, dial focus, a toggle): push the layout
    /// again; `notice` is an English line for the on-screen readout.
    Changed { notice: Option<String> },
    /// Settings changed from a key (profile, toggles): save them.
    Save,
}

/// What the on-screen readout shows.
#[derive(Clone, Debug, PartialEq)]
pub enum Shown {
    /// A value being turned (the host reads its current value).
    Value { target: String },
    /// A command that ran: English label and icon.
    Command { label: String, icon: Option<&'static str> },
    /// A surface notice (page or profile change, a toggle): English text.
    Notice { text: String },
}

#[derive(Clone, Debug)]
struct Gesture {
    target: String,
    last_input_ms: f64,
}

/// Learn mode: the next control touched is reported instead of acting.
#[derive(Clone, Debug, Default, PartialEq)]
enum Learn {
    #[default]
    Off,
    Armed,
    Got(String),
}

pub struct Runtime {
    settings: SurfaceSettings,
    context: String,
    /// The current page of each group.
    pages: BTreeMap<PageGroup, String>,
    /// The pages the context asks for itself (not inherited from `default`).
    context_pages: BTreeMap<PageGroup, String>,
    /// The pages last chosen by hand (or the profile's defaults): shown again when a context
    /// that asks for nothing special comes back.
    manual: BTreeMap<PageGroup, String>,
    /// The value the dials move instead of their own (a key bound to a value was pressed).
    focus: Option<String>,
    held: Vec<(String, Modifier)>,
    accel: Accelerator,
    pending: BTreeMap<String, f64>,
    gesture: Option<Gesture>,
    last_flush_ms: f64,
    learn: Learn,
    shown: Option<(Shown, f64)>,
    revision: u64,
}

impl Runtime {
    pub fn new(settings: SurfaceSettings) -> Runtime {
        let mut r = Runtime {
            settings,
            context: "default".into(),
            pages: BTreeMap::new(),
            context_pages: BTreeMap::new(),
            manual: BTreeMap::new(),
            focus: None,
            held: Vec::new(),
            accel: Accelerator::default(),
            pending: BTreeMap::new(),
            gesture: None,
            last_flush_ms: f64::NEG_INFINITY,
            learn: Learn::Off,
            shown: None,
            revision: 0,
        };
        r.settings.sanitize();
        r.reset_pages();
        r
    }

    pub fn settings(&self) -> &SurfaceSettings {
        &self.settings
    }

    /// Change settings (the editor), then [`Runtime::settings_changed`].
    pub fn settings_mut(&mut self) -> &mut SurfaceSettings {
        &mut self.settings
    }

    /// Settings were edited: clamp them, keep pages that still exist.
    pub fn settings_changed(&mut self) {
        self.settings.sanitize();
        let valid = |p: &Profile, g: PageGroup, n: &String| p.page(n).is_some_and(|pg| pg.in_group(g));
        if let Some(p) = self.settings.active_profile().cloned() {
            self.manual.retain(|g, n| valid(&p, *g, n));
            self.pages.retain(|g, n| valid(&p, *g, n));
        }
        let ctx = std::mem::take(&mut self.context);
        self.apply_context(&ctx);
        self.revision += 1;
    }

    /// Replace all settings (a file was imported or reloaded).
    pub fn replace_settings(&mut self, settings: SurfaceSettings) {
        self.settings = settings;
        self.settings.sanitize();
        self.reset_pages();
        self.revision += 1;
    }

    /// Bumped whenever the layout may have changed (pages, focus, profile, settings).
    pub fn revision(&self) -> u64 {
        self.revision
    }

    pub fn profile(&self) -> Option<&Profile> {
        self.settings.active_profile()
    }

    pub fn context(&self) -> &str {
        &self.context
    }

    /// The current page of each group.
    pub fn pages(&self) -> &BTreeMap<PageGroup, String> {
        &self.pages
    }

    /// The value the dials move instead of their own, if a key put one there.
    pub fn focus(&self) -> Option<&str> {
        self.focus.as_deref()
    }

    /// Modifiers held right now.
    pub fn held(&self) -> Vec<Modifier> {
        self.held.iter().map(|(_, m)| *m).collect()
    }

    /// The value being turned, if a gesture is in progress.
    pub fn turning(&self) -> Option<&str> {
        self.gesture.as_ref().map(|g| g.target.as_str())
    }

    /// While turning, values for `target` aren't sent back until every tick so far has been
    /// applied (TactileBridge's echo suppression; the readout never jumps back).
    pub fn holds_feedback(&self, target: &str) -> bool {
        self.settings.transport.echo_suppression && self.pending.get(target).is_some_and(|d| *d != 0.0)
    }

    /// What the on-screen readout shows at `now_ms`, and since when (it fades after the
    /// display time).
    pub fn shown(&self, now_ms: f64) -> Option<(&Shown, f64)> {
        let (s, at) = self.shown.as_ref()?;
        let age = now_ms - at;
        (self.settings.osd.enabled && age.is_finite() && age < f64::from(self.settings.osd.display_ms) + 400.0).then_some((s, *at))
    }

    fn show(&mut self, s: Shown, now_ms: f64) {
        let commands_ok = self.settings.osd.show_commands || matches!(s, Shown::Value { .. });
        if commands_ok {
            self.shown = Some((s, now_ms));
        }
    }

    // ------------------------------------------------------------------------------- learning

    /// The next control touched is reported by [`Runtime::take_learned`] instead of acting.
    pub fn arm_learn(&mut self) {
        self.learn = Learn::Armed;
    }

    pub fn cancel_learn(&mut self) {
        self.learn = Learn::Off;
    }

    pub fn learning(&self) -> bool {
        self.learn == Learn::Armed
    }

    /// The control learnt, once.
    pub fn take_learned(&mut self) -> Option<String> {
        match std::mem::take(&mut self.learn) {
            Learn::Got(c) => Some(c),
            other => {
                self.learn = other;
                None
            }
        }
    }

    // ------------------------------------------------------------------------------- pages

    fn reset_pages(&mut self) {
        self.manual = self.profile().map(|p| p.pages_for_context("default")).unwrap_or_default();
        self.pages.clear();
        self.focus = None;
        let ctx = std::mem::take(&mut self.context);
        self.apply_context(if ctx.is_empty() { "default" } else { &ctx });
    }

    fn apply_context(&mut self, key: &str) {
        self.context = key.to_string();
        let Some(p) = self.profile().cloned() else {
            self.pages.clear();
            self.context_pages.clear();
            return;
        };
        self.context_pages = p.context_pages(key, false);
        let defaults = p.pages_for_context("default");
        for g in PageGroup::ALL {
            let follow = self.settings.follow_context.then(|| self.context_pages.get(&g)).flatten();
            let keep = (!self.settings.follow_context).then(|| self.pages.get(&g)).flatten();
            let name =
                follow.or(keep).or(self.manual.get(&g)).or(defaults.get(&g)).cloned().or_else(|| p.pages_in(g).next().map(|pg| pg.name.clone()));
            match name {
                Some(n) => {
                    self.pages.insert(g, n);
                }
                None => {
                    self.pages.remove(&g);
                }
            }
        }
    }

    /// What the app shows changed (`crate::context::context_key`). Pages follow when Follow
    /// Context is on; the dial focus is dropped either way (as TactileBridge did on a tool
    /// change). Returns whether anything changed.
    pub fn set_context(&mut self, key: &str) -> bool {
        if key == self.context {
            return false;
        }
        let before = (self.pages.clone(), self.focus.clone());
        self.focus = None;
        self.apply_context(key);
        let changed = before != (self.pages.clone(), self.focus.clone());
        if changed {
            self.revision += 1;
        }
        changed
    }

    fn page_step(&mut self, group: Option<PageGroup>, step: i32, now_ms: f64) -> Vec<Effect> {
        let Some(p) = self.profile().cloned() else { return Vec::new() };
        let groups: Vec<PageGroup> = group.map_or(PageGroup::ALL.to_vec(), |g| vec![g]);
        let mut names = Vec::new();
        for g in groups {
            if let Some(next) = p.step_page(g, self.pages.get(&g).map(String::as_str), step) {
                self.pages.insert(g, next.clone());
                self.manual.insert(g, next.clone());
                names.push(next);
            }
        }
        if names.is_empty() {
            return Vec::new();
        }
        names.dedup();
        self.focus = None;
        self.revision += 1;
        let text = names.join(" · ");
        self.show(Shown::Notice { text: text.clone() }, now_ms);
        vec![Effect::Changed { notice: Some(text) }]
    }

    fn goto_page(&mut self, name: &str, now_ms: f64) -> Vec<Effect> {
        let Some(page) = self.profile().and_then(|p| p.page(name)).cloned() else { return Vec::new() };
        for g in PageGroup::ALL {
            if page.in_group(g) {
                self.pages.insert(g, page.name.clone());
                self.manual.insert(g, page.name.clone());
            }
        }
        self.focus = None;
        self.revision += 1;
        self.show(Shown::Notice { text: page.name.clone() }, now_ms);
        vec![Effect::Changed { notice: Some(page.name) }]
    }

    fn switch_profile(&mut self, name: Option<&str>, now_ms: f64) -> Vec<Effect> {
        let names: Vec<String> = self.settings.profiles.iter().map(|p| p.name.clone()).collect();
        let target = match name {
            Some(n) => names.iter().find(|p| *p == n).cloned(),
            None => {
                let i = names.iter().position(|p| *p == self.settings.profile).map_or(0, |i| i + 1);
                names.get(i % names.len().max(1)).cloned()
            }
        };
        let Some(target) = target else { return Vec::new() };
        let mut out = self.interrupt();
        self.settings.profile = target.clone();
        self.reset_pages();
        self.revision += 1;
        self.show(Shown::Notice { text: target.clone() }, now_ms);
        out.push(Effect::Save);
        out.push(Effect::Changed { notice: Some(target) });
        out
    }

    // ------------------------------------------------------------------------------- bindings

    /// The page the device shows as its key page `n` (1-based): the context's own key page while
    /// one is active, else the profile's `n`-th key page.
    pub fn device_key_page(&self, n: usize) -> Option<&Page> {
        let p = self.profile()?;
        if self.settings.follow_context
            && let Some(name) = self.context_pages.get(&PageGroup::Keys)
        {
            return p.page(name);
        }
        p.pages_in(PageGroup::Keys).nth(n.checked_sub(1)?)
    }

    /// How many key pages the profile has.
    pub fn key_page_count(&self) -> usize {
        self.profile().map_or(0, |p| p.pages_in(PageGroup::Keys).count())
    }

    /// The binding of `control` now (`page`: the device's own key page, see [`Input::Press`]).
    /// Encoders bound to nothing, or a value, follow the dial focus.
    pub fn binding(&self, control: &str, page: Option<usize>) -> Option<Binding> {
        let p = self.profile()?;
        if is_encoder(control)
            && let Some(f) = &self.focus
        {
            let own = p.binding(control, &self.pages);
            // a dial focus moves the dial (the first encoder) only; the others keep their values
            if control == "dial" || own.is_none() {
                return Some(Binding::value(f));
            }
        }
        if let (Some(n), PageGroup::Keys) = (page, group_of(control)) {
            return self.device_key_page(n).and_then(|pg| pg.bindings.get(control)).or_else(|| p.base.get(control)).cloned();
        }
        p.binding(control, &self.pages).cloned()
    }

    // ------------------------------------------------------------------------------- input

    /// Handle one input event at `now_ms`.
    pub fn input(&mut self, input: Input, now_ms: f64) -> Vec<Effect> {
        if !self.settings.enabled {
            return Vec::new();
        }
        if self.learn == Learn::Armed {
            let control = match &input {
                Input::Turn { control, .. } | Input::Press { control, .. } => Some(control.clone()),
                Input::Release { .. } => None,
            };
            if let Some(c) = control {
                self.learn = Learn::Got(c);
                self.revision += 1;
                return vec![Effect::Changed { notice: None }];
            }
            return Vec::new();
        }
        match input {
            Input::Turn { control, ticks } => self.turn(&control, ticks, now_ms),
            Input::Press { control, page } => self.press(&control, page, now_ms),
            Input::Release { control } => {
                let before = self.held.len();
                self.held.retain(|(c, _)| *c != control);
                if self.held.len() != before {
                    self.revision += 1;
                    vec![Effect::Changed { notice: None }]
                } else {
                    Vec::new()
                }
            }
        }
    }

    fn turn(&mut self, control: &str, ticks: i32, now_ms: f64) -> Vec<Effect> {
        if ticks == 0 {
            return Vec::new();
        }
        match self.binding(control, None) {
            Some(Binding::Value { target }) => self.turn_value(&target, ticks, now_ms),
            Some(Binding::PageNext { group }) => self.page_step(group, ticks.signum(), now_ms),
            Some(Binding::PagePrevious { group }) => self.page_step(group, -ticks.signum(), now_ms),
            // other bindings act once per turn event, clockwise only
            Some(b) if ticks > 0 => self.perform(control, &b, now_ms),
            _ => Vec::new(),
        }
    }

    fn turn_value(&mut self, target: &str, ticks: i32, now_ms: f64) -> Vec<Effect> {
        let Some(spec) = target_spec(target) else { return Vec::new() };
        let mut out = Vec::new();
        if self.gesture.as_ref().is_some_and(|g| g.target != target) {
            out.extend(self.finish_gesture());
        }
        if self.gesture.is_none() {
            out.push(Effect::Begin { target: target.to_string() });
        }
        self.gesture = Some(Gesture { target: target.to_string(), last_input_ms: now_ms });
        let tuning = self.settings.tuning_of(target);
        let motion = self.settings.motion.clone();
        let acc = self.accel.acceleration(target, now_ms, &tuning, &motion);
        let held = self.held();
        let d = delta(ticks, spec.step, acc, modifier_factor(&held, &motion), &tuning, &motion, spec.max_delta());
        let lim = spec.max_delta();
        let p = self.pending.entry(target.to_string()).or_insert(0.0);
        *p = (*p + d).clamp(-lim, lim);
        self.show(Shown::Value { target: target.to_string() }, now_ms);
        out
    }

    fn press(&mut self, control: &str, page: Option<usize>, now_ms: f64) -> Vec<Effect> {
        if is_encoder(control) {
            // a dial press: its own binding (`dial.press`), else reset what the dial moves
            let own = format!("{control}.press");
            if let Some(b) = self.binding(&own, None) {
                return self.perform(&own, &b, now_ms);
            }
            return match self.binding(control, None) {
                Some(Binding::Value { target }) => self.reset(&target, now_ms),
                _ => Vec::new(),
            };
        }
        match self.binding(control, page) {
            Some(b) => self.perform(control, &b, now_ms),
            None => Vec::new(),
        }
    }

    fn reset(&mut self, target: &str, now_ms: f64) -> Vec<Effect> {
        let mut out = self.interrupt();
        if target_spec(target).is_some() {
            self.show(Shown::Value { target: target.to_string() }, now_ms);
            out.push(Effect::Reset { target: target.to_string() });
        }
        out
    }

    /// Do what `b` does, as if a key bound to it (`control`) were pressed (commands that page,
    /// switch profiles or toggle without a key).
    pub fn perform(&mut self, control: &str, b: &Binding, now_ms: f64) -> Vec<Effect> {
        match b {
            Binding::Value { target } => {
                // the adjustment dial toggle: press to put this value on the dial, again to give
                // the dial back
                self.focus = if self.focus.as_deref() == Some(target) { None } else { Some(target.clone()) };
                self.revision += 1;
                let text = match &self.focus {
                    Some(t) => format!("Dial: {}", target_spec(t).map_or(t.as_str(), |s| s.label)),
                    None => "Dial: page".to_string(),
                };
                self.show(Shown::Notice { text: text.clone() }, now_ms);
                vec![Effect::Changed { notice: Some(text) }]
            }
            Binding::Command { command, params } => {
                self.show(Shown::Command { label: b.describe(), icon: crate::icons::icon_for(b) }, now_ms);
                // a command may change the photo: what the dial did so far goes to this one first
                let mut out = self.interrupt();
                out.push(Effect::Command { command: command.clone(), params: params.clone() });
                out
            }
            Binding::Modifier { modifier } => {
                if !self.held.iter().any(|(c, m)| c == control && m == modifier) {
                    self.held.push((control.to_string(), *modifier));
                }
                self.revision += 1;
                vec![Effect::Changed { notice: None }]
            }
            Binding::PageNext { group } => self.page_step(*group, 1, now_ms),
            Binding::PagePrevious { group } => self.page_step(*group, -1, now_ms),
            Binding::Page { page } => self.goto_page(page, now_ms),
            Binding::Profile { profile } => self.switch_profile(Some(profile), now_ms),
            Binding::ProfileNext => self.switch_profile(None, now_ms),
            Binding::Reset => match self.binding("dial", None) {
                Some(Binding::Value { target }) => self.reset(&target, now_ms),
                _ => Vec::new(),
            },
            Binding::ApplyToSelected => {
                self.settings.apply_to_selected = !self.settings.apply_to_selected;
                self.revision += 1;
                let text = format!("Apply to all selected: {}", if self.settings.apply_to_selected { "On" } else { "Off" });
                self.show(Shown::Notice { text: text.clone() }, now_ms);
                vec![Effect::Save, Effect::Changed { notice: Some(text) }]
            }
            Binding::FollowContext => {
                self.settings.follow_context = !self.settings.follow_context;
                let ctx = self.context.clone();
                self.apply_context(&ctx);
                self.revision += 1;
                let text = format!("Pages follow the app: {}", if self.settings.follow_context { "On" } else { "Off" });
                self.show(Shown::Notice { text: text.clone() }, now_ms);
                vec![Effect::Save, Effect::Changed { notice: Some(text) }]
            }
        }
    }

    // ------------------------------------------------------------------------------- flushing

    fn finish_gesture(&mut self) -> Vec<Effect> {
        let Some(g) = self.gesture.take() else { return Vec::new() };
        let mut out = Vec::new();
        if let Some(d) = self.pending.remove(&g.target)
            && d != 0.0
        {
            out.push(Effect::Adjust { target: g.target.clone(), delta: d });
        }
        self.accel.reset(&g.target);
        out.push(Effect::End { target: g.target, apply_to_selected: self.settings.apply_to_selected });
        out
    }

    /// Apply gathered dial movement (at most once per flush interval) and end a gesture that has
    /// been idle for the gesture time. Call every frame.
    pub fn flush(&mut self, now_ms: f64) -> Vec<Effect> {
        let mut out = Vec::new();
        if !now_ms.is_finite() {
            return out;
        }
        let interval = f64::from(self.settings.transport.flush_ms);
        if !self.pending.is_empty() && (now_ms - self.last_flush_ms >= interval || now_ms < self.last_flush_ms) {
            for (target, d) in std::mem::take(&mut self.pending) {
                if d != 0.0 {
                    out.push(Effect::Adjust { target, delta: d });
                }
            }
            self.last_flush_ms = now_ms;
        }
        let idle = f64::from(self.settings.transport.gesture_end_ms);
        if self.pending.is_empty() && self.gesture.as_ref().is_some_and(|g| now_ms - g.last_input_ms >= idle) {
            out.extend(self.finish_gesture());
        }
        out
    }

    /// End any gesture now, applying what is pending (the active photo changed, the app took
    /// over the edit, the surface went away).
    pub fn interrupt(&mut self) -> Vec<Effect> {
        let mut out: Vec<Effect> = std::mem::take(&mut self.pending)
            .into_iter()
            .filter(|(_, d)| *d != 0.0)
            .map(|(target, delta)| Effect::Adjust { target, delta })
            .collect();
        // nothing is pending now: this only ends the gesture
        out.extend(self.finish_gesture());
        out
    }

    /// Does anything still need frames (pending movement, an open gesture, a fading readout)?
    pub fn busy(&self, now_ms: f64) -> bool {
        !self.pending.is_empty() || self.gesture.is_some() || self.shown(now_ms).is_some()
    }

    /// The modifier held on a control, if any (keys show it lit).
    pub fn held_on(&self, control: &str) -> Option<Modifier> {
        self.held.iter().find(|(c, _)| c == control).map(|(_, m)| *m)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::devices::PageGroup;
    use crate::profile::Page;

    fn settings() -> SurfaceSettings {
        let mut p = Profile { name: "T".into(), ..Default::default() };
        p.pages.push(Page::new(
            "Cull",
            &[PageGroup::Keys],
            [("key.1", Binding::command("library.next")), ("key.2", Binding::value("light.contrast"))],
        ));
        p.pages.push(Page::new("Edit", &[PageGroup::Keys], [("key.1", Binding::command("develop.copy"))]));
        p.pages.push(Page::new("Light", &[PageGroup::Dials], [("dial", Binding::value("light.exposure")), ("roller", Binding::value("wb.temp"))]));
        p.pages.push(Page::new("HDR", &[PageGroup::Dials], [("dial", Binding::value("light.highlights"))]));
        p.pages.push(Page::new("Crop", &[], [("key.1", Binding::command("crop.rotateAspect")), ("dial", Binding::value("crop.angle"))]));
        p.base.insert("pad.1".into(), Binding::Modifier { modifier: Modifier::Fine });
        p.base.insert("pad.2".into(), Binding::PageNext { group: Some(PageGroup::Dials) });
        p.base.insert("pad.3".into(), Binding::ApplyToSelected);
        p.base.insert("pad.4".into(), Binding::Reset);
        p.contexts.insert("default".into(), vec!["Cull".into(), "Light".into()]);
        p.contexts.insert("crop".into(), vec!["Crop".into()]);
        SurfaceSettings { profiles: vec![p], profile: "T".into(), enabled: true, ..SurfaceSettings::default() }
    }

    fn turn(c: &str, t: i32) -> Input {
        Input::Turn { control: c.into(), ticks: t }
    }
    fn press(c: &str) -> Input {
        Input::Press { control: c.into(), page: None }
    }
    fn release(c: &str) -> Input {
        Input::Release { control: c.into() }
    }
    fn adjusts(e: &[Effect]) -> f64 {
        e.iter().map(|e| if let Effect::Adjust { delta, .. } = e { *delta } else { 0.0 }).sum()
    }

    #[test]
    fn ticks_coalesce_into_one_adjust_per_flush_and_one_gesture() {
        let mut r = Runtime::new(settings());
        let e = r.input(turn("dial", 1), 0.0);
        assert_eq!(e, [Effect::Begin { target: "light.exposure".into() }]);
        assert!(r.input(turn("dial", 1), 300.0).is_empty(), "no second Begin");
        assert!(r.holds_feedback("light.exposure"));
        let f = r.flush(300.0);
        assert!((adjusts(&f) - 0.02).abs() < 1e-12, "{f:?}");
        assert!(!r.holds_feedback("light.exposure"));
        // within the flush interval nothing more goes out
        r.input(turn("dial", 1), 305.0); // 5 ms after the last tick: the full 10× ceiling
        assert!(r.flush(306.0).is_empty());
        assert!((adjusts(&r.flush(320.0)) - 0.1).abs() < 1e-12);
        // idle past the gesture time: one End
        assert!(r.flush(1000.0).is_empty());
        let end = r.flush(305.0 + 800.0);
        assert_eq!(end, [Effect::End { target: "light.exposure".into(), apply_to_selected: false }]);
        assert!(r.flush(5000.0).is_empty());
        assert!(r.turning().is_none());
    }

    #[test]
    fn fast_turning_accelerates() {
        let mut r = Runtime::new(settings());
        r.input(turn("dial", 1), 0.0);
        r.input(turn("dial", 1), 5.0); // 5 ms after: full ceiling (10×)
        let f = r.flush(10.0);
        assert!((adjusts(&f) - 0.11).abs() < 1e-9, "{f:?}");
    }

    #[test]
    fn switching_dials_ends_the_first_gesture() {
        let mut r = Runtime::new(settings());
        r.input(turn("dial", 2), 0.0);
        let e = r.input(turn("roller", 1), 10.0);
        assert_eq!(
            e,
            [
                Effect::Adjust { target: "light.exposure".into(), delta: 0.02 },
                Effect::End { target: "light.exposure".into(), apply_to_selected: false },
                Effect::Begin { target: "wb.temp".into() },
            ]
        );
    }

    #[test]
    fn modifiers_scale_steps_while_held() {
        let mut r = Runtime::new(settings());
        r.input(press("pad.1"), 0.0);
        assert_eq!(r.held(), [Modifier::Fine]);
        r.input(turn("dial", 4), 1000.0);
        assert!((adjusts(&r.flush(1000.0)) - 0.01).abs() < 1e-12, "4 ticks × 0.01 × ¼");
        r.input(release("pad.1"), 1001.0);
        assert!(r.held().is_empty());
    }

    #[test]
    fn a_key_bound_to_a_value_puts_it_on_the_dial_and_back() {
        let mut r = Runtime::new(settings());
        let rev = r.revision();
        r.input(press("key.2"), 0.0);
        assert_eq!(r.focus(), Some("light.contrast"));
        assert!(r.revision() > rev);
        assert_eq!(r.binding("dial", None), Some(Binding::value("light.contrast")));
        assert_eq!(r.binding("roller", None), Some(Binding::value("wb.temp")), "the roller keeps its own");
        assert_eq!(r.input(turn("dial", 1), 10.0), [Effect::Begin { target: "light.contrast".into() }]);
        r.input(press("key.2"), 20.0);
        assert_eq!(r.focus(), None);
        // a context change drops the focus too
        r.input(press("key.2"), 30.0);
        r.set_context("crop");
        assert_eq!(r.focus(), None);
    }

    #[test]
    fn pages_follow_context_and_come_back_to_the_hand_picked_ones() {
        let mut r = Runtime::new(settings());
        assert_eq!(r.pages()[&PageGroup::Dials], "Light");
        r.input(press("pad.2"), 0.0); // next dial page
        assert_eq!(r.pages()[&PageGroup::Dials], "HDR");
        assert!(r.set_context("crop"));
        assert_eq!((r.pages()[&PageGroup::Keys].as_str(), r.pages()[&PageGroup::Dials].as_str()), ("Crop", "Crop"));
        assert_eq!(r.binding("key.1", Some(2)), Some(Binding::command("crop.rotateAspect")), "a context page fills every device page");
        r.set_context("develop.light");
        assert_eq!((r.pages()[&PageGroup::Keys].as_str(), r.pages()[&PageGroup::Dials].as_str()), ("Cull", "HDR"));
        assert_eq!(r.binding("key.1", Some(2)), Some(Binding::command("develop.copy")), "device page 2 = second key page");
        assert_eq!(r.binding("key.1", Some(9)), None);
        // follow off: contexts don't move pages
        r.input(Input::Press { control: "x".into(), page: None }, 0.0);
        r.settings_mut().follow_context = false;
        r.settings_changed();
        r.set_context("crop");
        assert_eq!(r.pages()[&PageGroup::Keys], "Cull");
    }

    #[test]
    fn commands_toggles_and_dial_press() {
        let mut r = Runtime::new(settings());
        assert_eq!(r.input(press("key.1"), 0.0), [Effect::Command { command: "library.next".into(), params: Value::Null }]);
        assert!(matches!(r.shown(100.0), Some((Shown::Command { .. }, _))));
        assert!(r.shown(100_000.0).is_none(), "the readout times out");
        let e = r.input(press("pad.3"), 0.0);
        assert!(r.settings().apply_to_selected && e.contains(&Effect::Save));
        r.input(turn("dial", 1), 10.0);
        let e = r.interrupt();
        assert_eq!(e.last(), Some(&Effect::End { target: "light.exposure".into(), apply_to_selected: true }));
        assert_eq!(r.input(press("dial"), 20.0), [Effect::Reset { target: "light.exposure".into() }]);
        assert_eq!(r.input(press("pad.4"), 30.0), [Effect::Reset { target: "light.exposure".into() }]);
        assert!(r.input(press("key.9"), 0.0).is_empty(), "unbound");
    }

    #[test]
    fn interrupt_applies_pending_and_ends() {
        let mut r = Runtime::new(settings());
        r.input(turn("dial", 3), 0.0);
        let e = r.interrupt();
        assert_eq!(e.len(), 2);
        assert!((adjusts(&e) - 0.03).abs() < 1e-12);
        assert!(r.interrupt().is_empty());
        assert!(!r.busy(1e9));
    }

    #[test]
    fn learn_mode_reports_the_control_instead_of_acting() {
        let mut r = Runtime::new(settings());
        r.arm_learn();
        assert!(r.learning());
        assert_eq!(r.input(press("key.1"), 0.0), [Effect::Changed { notice: None }]);
        assert_eq!(r.take_learned().as_deref(), Some("key.1"));
        assert_eq!(r.take_learned(), None);
        assert_eq!(r.input(press("key.1"), 0.0).len(), 1, "acts again");
    }

    #[test]
    fn disabled_surfaces_do_nothing_and_hostile_input_is_harmless() {
        let mut r = Runtime::new(settings());
        r.input(turn("dial", i32::MAX), 0.0);
        let f = r.flush(0.0);
        assert!(adjusts(&f).abs() <= 10.0, "limited to the exposure range span");
        assert!(r.input(turn("nope", 5), 0.0).is_empty());
        assert!(r.input(turn("dial", 0), 0.0).is_empty());
        assert!(r.flush(f64::NAN).is_empty());
        r.settings_mut().enabled = false;
        assert!(r.input(press("key.1"), 0.0).is_empty());
    }

    #[test]
    fn profiles_switch_and_pages_reset() {
        let mut s = settings();
        let mut q = s.profiles[0].clone();
        q.name = "Q".into();
        s.profiles.push(q);
        let mut r = Runtime::new(s);
        r.input(press("pad.2"), 0.0);
        let e = r.perform("x", &Binding::ProfileNext, 10.0);
        assert!(e.contains(&Effect::Save));
        assert_eq!(r.settings().profile, "Q");
        assert_eq!(r.pages()[&PageGroup::Dials], "Light", "a new profile starts on its default pages");
        r.perform("x", &Binding::Profile { profile: "T".into() }, 20.0);
        assert_eq!(r.settings().profile, "T");
        assert!(r.perform("x", &Binding::Profile { profile: "missing".into() }, 30.0).is_empty());
        let e = r.perform("x", &Binding::Page { page: "Edit".into() }, 40.0);
        assert_eq!(e, [Effect::Changed { notice: Some("Edit".into()) }]);
        assert_eq!(r.pages()[&PageGroup::Keys], "Edit");
    }
}
