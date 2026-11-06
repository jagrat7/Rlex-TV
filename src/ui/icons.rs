//! The icons Rlex draws, as Lucide-style SVG paths on a 24-point grid.
//! Rune rasterizes them into its glyph atlas, tinted by the text colour.

use rune::Icon;

/// Outline icons: each path stroked 2 points wide with round ends.
macro_rules! stroked {
    ($($name:ident: $($d:literal),+;)*) => {$(
        pub static $name: Icon = Icon {
            name: concat!("rlex-", stringify!($name)),
            view: 24.0,
            body: concat!($(
                "<path d=\"", $d,
                "\" stroke=\"currentColor\" stroke-width=\"2\" stroke-linecap=\"round\" stroke-linejoin=\"round\"/>"
            ),+),
        };
    )*};
}

/// Solid icons: each path filled.
macro_rules! filled {
    ($($name:ident: $($d:literal),+;)*) => {$(
        pub static $name: Icon = Icon {
            name: concat!("rlex-", stringify!($name)),
            view: 24.0,
            body: concat!($("<path d=\"", $d, "\" fill=\"currentColor\"/>"),+),
        };
    )*};
}

filled! {
    PLAY: "M6.5 4.4v15.2a1.2 1.2 0 0 0 1.84 1.01l11.9-7.6a1.2 1.2 0 0 0 0-2.02l-11.9-7.6A1.2 1.2 0 0 0 6.5 4.4z";
    PAUSE: "M6 5a1 1 0 0 1 1-1h2.5a1 1 0 0 1 1 1v14a1 1 0 0 1-1 1H7a1 1 0 0 1-1-1z",
           "M13.5 5a1 1 0 0 1 1-1H17a1 1 0 0 1 1 1v14a1 1 0 0 1-1 1h-2.5a1 1 0 0 1-1-1z";
}

stroked! {
    INFO: "M2 12a10 10 0 1 0 20 0a10 10 0 1 0 -20 0", "M12 16v-4", "M12 8h.01";
    PLUS: "M5 12h14", "M12 5v14";
    CHECK: "M20 6 9 17l-5-5";
    CIRCLE: "M2 12a10 10 0 1 0 20 0a10 10 0 1 0 -20 0";
    CIRCLE_CHECK: "M2 12a10 10 0 1 0 20 0a10 10 0 1 0 -20 0", "m9 12 2 2 4-4";
    ALERT: "M2 12a10 10 0 1 0 20 0a10 10 0 1 0 -20 0", "M12 8v4", "M12 16h.01";
    CHEVRON_LEFT: "m15 18-6-6 6-6";
    CHEVRON_RIGHT: "m9 18 6-6-6-6";
    CHEVRON_DOWN: "m6 9 6 6 6-6";
    ARROW_LEFT: "m12 19-7-7 7-7", "M19 12H5";
    CLOSE: "M18 6 6 18", "m6 6 12 12";
    SEARCH: "M3 11a8 8 0 1 0 16 0a8 8 0 1 0 -16 0", "m21 21-4.3-4.3";
    SEARCH_X: "M3 11a8 8 0 1 0 16 0a8 8 0 1 0 -16 0", "m21 21-4.3-4.3", "m13.5 8.5-5 5", "m8.5 8.5 5 5";
    SETTINGS: "M12.22 2h-.44a2 2 0 0 0-2 2v.18a2 2 0 0 1-1 1.73l-.43.25a2 2 0 0 1-2 0l-.15-.08a2 2 0 0 0-2.73.73l-.22.38a2 2 0 0 0 .73 2.73l.15.1a2 2 0 0 1 1 1.72v.51a2 2 0 0 1-1 1.74l-.15.09a2 2 0 0 0-.73 2.73l.22.38a2 2 0 0 0 2.73.73l.15-.08a2 2 0 0 1 2 0l.43.25a2 2 0 0 1 1 1.73V20a2 2 0 0 0 2 2h.44a2 2 0 0 0 2-2v-.18a2 2 0 0 1 1-1.73l.43-.25a2 2 0 0 1 2 0l.15.08a2 2 0 0 0 2.73-.73l.22-.39a2 2 0 0 0-.73-2.73l-.15-.08a2 2 0 0 1-1-1.74v-.5a2 2 0 0 1 1-1.74l.15-.09a2 2 0 0 0 .73-2.73l-.22-.38a2 2 0 0 0-2.73-.73l-.15.08a2 2 0 0 1-2 0l-.43-.25a2 2 0 0 1-1-1.73V4a2 2 0 0 0-2-2z",
              "M9 12a3 3 0 1 0 6 0a3 3 0 1 0 -6 0";
    FOLDER: "M20 20a2 2 0 0 0 2-2V8a2 2 0 0 0-2-2h-7.9a2 2 0 0 1-1.69-.9L9.6 3.9A2 2 0 0 0 7.93 3H4a2 2 0 0 0-2 2v13a2 2 0 0 0 2 2Z";
    FOLDER_PLUS: "M20 20a2 2 0 0 0 2-2V8a2 2 0 0 0-2-2h-7.9a2 2 0 0 1-1.69-.9L9.6 3.9A2 2 0 0 0 7.93 3H4a2 2 0 0 0-2 2v13a2 2 0 0 0 2 2Z",
                 "M12 10v6", "M9 13h6";
    FOLDER_OPEN: "m6 14 1.5-2.9A2 2 0 0 1 9.24 10H20a2 2 0 0 1 1.94 2.5l-1.54 6a2 2 0 0 1-1.95 1.5H4a2 2 0 0 1-2-2V5a2 2 0 0 1 2-2h3.9a2 2 0 0 1 1.69.9l.81 1.2a2 2 0 0 0 1.67.9H18a2 2 0 0 1 2 2v2";
    HARD_DRIVE: "M22 12H2", "M5.45 5.11 2 12v6a2 2 0 0 0 2 2h16a2 2 0 0 0 2-2v-6l-3.45-6.89A2 2 0 0 0 16.76 4H7.24a2 2 0 0 0-1.79 1.11z",
                "M6 16h.01", "M10 16h.01";
    GLOBE: "M2 12a10 10 0 1 0 20 0a10 10 0 1 0 -20 0", "M12 2a14.5 14.5 0 0 0 0 20 14.5 14.5 0 0 0 0-20", "M2 12h20";
    DOWNLOAD: "M21 15v4a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2v-4", "m7 10 5 5 5-5", "M12 15V3";
    TRASH: "M3 6h18", "M19 6v14c0 1-1 2-2 2H7c-1 0-2-1-2-2V6", "M8 6V4c0-1 1-2 2-2h4c1 0 2 1 2 2v2";
    REFRESH: "M3 12a9 9 0 0 1 9-9 9.75 9.75 0 0 1 6.74 2.74L21 8", "M21 3v5h-5",
             "M21 12a9 9 0 0 1-9 9 9.75 9.75 0 0 1-6.74-2.74L3 16", "M8 16H3v5";
    SPARKLES: "M9.94 15.5A2 2 0 0 0 8.5 14.06l-6.14-1.58a.5.5 0 0 1 0-.96L8.5 9.94A2 2 0 0 0 9.94 8.5l1.58-6.14a.5.5 0 0 1 .96 0L14.06 8.5A2 2 0 0 0 15.5 9.94l6.14 1.58a.5.5 0 0 1 0 .96L15.5 14.06a2 2 0 0 0-1.44 1.44l-1.58 6.14a.5.5 0 0 1-.96 0z",
              "M20 3v4", "M22 5h-4";
    EXTERNAL: "M15 3h6v6", "M10 14 21 3", "M18 13v6a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2V8a2 2 0 0 1 2-2h6";
    LOADER: "M21 12a9 9 0 1 1-6.22-8.56";
}
