//! The interface driven headlessly: real views, keys and presses, on an
//! empty backend (nothing read from disk, nothing fetched).

use rune::{Env, Key, KeyEvent, Mods, Runtime};

use super::{Action, App, Bg, DetailTarget, Load, Page, on_key, view};
use crate::addons::Meta;
use crate::state::Core;

fn runtime() -> Runtime<App> {
    let mut rt = Runtime::new(
        App::with(Core::empty(), Bg::detached()),
        view,
        (1280.0, 800.0),
        Env::new(true, 1.0),
    );
    rt.on_key = Some(on_key);
    rt.frame(0.0);
    rt
}

fn settle(rt: &mut Runtime<App>) {
    for _ in 0..30 {
        rt.frame(1.0 / 60.0);
    }
}

fn key(rt: &mut Runtime<App>, key: Key) {
    rt.key(KeyEvent {
        key,
        mods: Mods::default(),
    });
    settle(rt);
}

fn local(name: &str) -> DetailTarget {
    DetailTarget {
        kind: "movie".into(),
        id: format!("local:/films/{name}.mkv"),
        name: name.into(),
        poster: None,
    }
}

#[test]
fn typing_over_the_page_searches_and_escape_goes_back() {
    let mut rt = runtime();
    key(&mut rt, Key::Text("d".into()));
    assert_eq!(rt.state.nav.page, Page::Search);
    assert_eq!(rt.state.search.edit.text, "d");
    // The box has the keyboard now, so Esc is the field's: it clears it.
    key(&mut rt, Key::Escape);
    assert_eq!(rt.state.nav.page, Page::Home);
    assert!(rt.state.search.edit.text.is_empty());
}

#[test]
fn escape_closes_the_top_sheet_and_back_returns_from_settings() {
    let mut rt = runtime();
    rt.update(|s, tx| {
        s.act(Action::Open(local("One")), tx);
        s.act(Action::Open(local("Two")), tx);
    });
    settle(&mut rt);
    assert!(rt.find("sheet-layer").is_some());
    key(&mut rt, Key::Escape);
    assert_eq!(rt.state.nav.details.len(), 1);
    assert_eq!(rt.state.nav.details[0].target.name, "One");

    // "Add a source to play" parks the sheet on Settings; Back brings it back.
    rt.update(|s, tx| s.act(Action::AddSource, tx));
    settle(&mut rt);
    assert_eq!(rt.state.nav.page, Page::Settings);
    assert!(rt.state.nav.details.is_empty());
    key(&mut rt, Key::Escape);
    assert_eq!(rt.state.nav.page, Page::Home);
    assert_eq!(rt.state.nav.details.len(), 1);
}

#[test]
fn pressing_a_poster_opens_its_details() {
    let mut rt = runtime();
    let meta: Meta =
        serde_json::from_value(serde_json::json!({"id": "tt1", "type": "movie", "name": "First"}))
            .unwrap();
    rt.update(move |s, _| {
        s.sync_home();
        s.home.rows[0].items = Load::Ready(vec![meta]);
    });
    settle(&mut rt);
    let (x, y) = rt
        .find("home-0/scroll/#0/0/poster")
        .expect("the first poster is built");
    rt.pointer_down(x, y);
    rt.pointer_up(x, y);
    settle(&mut rt);
    let opened = rt.state.nav.details.last().map(|d| d.target.id.clone());
    assert_eq!(opened.as_deref(), Some("tt1"));
}

#[test]
fn every_page_builds_with_nothing_loaded() {
    let mut rt = runtime();
    for page in [
        Page::Library,
        Page::Downloads,
        Page::Settings,
        Page::Search,
        Page::Home,
    ] {
        rt.update(move |s, tx| s.act(Action::Go(page), tx));
        settle(&mut rt);
        assert!(rt.find(page.key()).is_some(), "{page:?} shows");
    }
}
