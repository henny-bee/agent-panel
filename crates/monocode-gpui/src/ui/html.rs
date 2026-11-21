//! Native web views over GPUI. Generated Mono HTML lives in an opaque sandbox
//! with no app bridge or network. An HTML file loads from its own folder, so
//! its relative CSS, scripts and images resolve as they do in a browser.
//!
//! WKWebView (macOS) and Edge WebView2 (Windows, through wry) draw above the
//! GPUI surface, so nothing GPUI paints can cover them. A view shows while
//! frames draw its element, and hides under a popover or modal.
#[cfg(windows)]
mod webview2;
#[cfg(not(target_os = "macos"))]
use gpui::div;
#[cfg(any(target_os = "macos", windows))]
use gpui::{Bounds, Pixels, canvas};
use gpui::{Context, Window, prelude::*};
#[cfg(target_os = "macos")]
use objc2::{
    DefinedClass, MainThreadMarker, MainThreadOnly, define_class, msg_send, rc::Retained,
    runtime::ProtocolObject,
};
#[cfg(target_os = "macos")]
use objc2_app_kit::{NSView, NSWindowOrderingMode};
#[cfg(target_os = "macos")]
use objc2_foundation::{NSObject, NSObjectProtocol};
#[cfg(target_os = "macos")]
use objc2_foundation::{NSPoint, NSRect, NSSize, NSString, NSURL};
#[cfg(target_os = "macos")]
use objc2_web_kit::{
    WKNavigationAction, WKNavigationActionPolicy, WKNavigationDelegate, WKNavigationType,
    WKWebView, WKWebViewConfiguration, WKWebsiteDataStore,
};
#[cfg(target_os = "macos")]
use raw_window_handle::{HasWindowHandle, RawWindowHandle};
use std::path::{Path, PathBuf};
#[cfg(any(target_os = "macos", windows))]
use std::{
    cell::RefCell,
    rc::{Rc, Weak},
    sync::Arc,
};
#[cfg(target_os = "macos")]
define_class!(
    #[unsafe(super(NSView))]
    #[name = "MonocodeArtifactClip"]
    #[thread_kind = MainThreadOnly]
    #[ivars = std::cell::Cell<bool>]
    struct ArtifactClip;
    unsafe impl NSObjectProtocol for ArtifactClip {}
    impl ArtifactClip {
        #[unsafe(method(isFlipped))]
        fn flipped(&self) -> bool { true }

        #[unsafe(method_id(hitTest:))]
        fn hit_test(&self, point: NSPoint) -> Option<Retained<NSView>> {
            if !self.ivars().get() {
                None
            } else {
                unsafe { msg_send![super(self), hitTest: point] }
            }
        }
    }
);
// The ivars hold whether the page is a file, which may open other local files.
#[cfg(target_os = "macos")]
define_class!(
    #[unsafe(super(NSObject))]
    #[name = "MonocodeArtifactNavigation"]
    #[thread_kind=MainThreadOnly]
    #[ivars=(gpui::AsyncApp, bool)]
    struct NavigationGuard;
    unsafe impl NSObjectProtocol for NavigationGuard {}
    unsafe impl WKNavigationDelegate for NavigationGuard {
        #[unsafe(method(webView:decidePolicyForNavigationAction:decisionHandler:))]
        unsafe fn decide(
            &self,
            _: &WKWebView,
            action: &WKNavigationAction,
            reply: &block2::DynBlock<dyn Fn(WKNavigationActionPolicy)>,
        ) {
            let url = unsafe { action.request().URL() }.and_then(|url| url.absoluteString());
            if unsafe { action.navigationType() } == WKNavigationType::LinkActivated
                && let Some(url) = url.as_ref()
                && (url.to_string().starts_with("https://")
                    || url.to_string().starts_with("http://"))
            {
                let url = url.to_string();
                self.ivars().0.clone().update(|cx| cx.open_url(&url));
            }
            let file = self.ivars().1;
            let allowed = url.is_none_or(|url| {
                let s = url.to_string();
                s.starts_with("about:blank")
                    || s.starts_with("about:srcdoc")
                    || (file && s.starts_with("file://"))
            });
            reply.call((if allowed {
                WKNavigationActionPolicy::Allow
            } else {
                WKNavigationActionPolicy::Cancel
            },));
        }
    }
);
/// What a native view shows: the sandboxed document, or a file and the
/// number of times it was reloaded.
#[cfg(any(target_os = "macos", windows))]
#[derive(Clone)]
enum Page {
    Markup(Arc<str>),
    File(PathBuf, u32),
}
#[cfg(any(target_os = "macos", windows))]
impl Page {
    fn same(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::Markup(a), Self::Markup(b)) => Arc::ptr_eq(a, b),
            (Self::File(a, x), Self::File(b, y)) => a == b && x == y,
            _ => false,
        }
    }
}
#[cfg(target_os = "macos")]
struct Hosted {
    window: gpui::AnyWindowHandle,
    clip: Retained<ArtifactClip>,
    web: Retained<WKWebView>,
    _guard: Retained<NavigationGuard>,
    page: Page,
    live: bool,
    rect: Option<Bounds<Pixels>>,
    shown: bool,
}
#[cfg(windows)]
struct Hosted {
    window: gpui::AnyWindowHandle,
    web: webview2::Web,
    page: Page,
    bounds: Option<wry::Rect>,
    zoom: f32,
    /// Frames draw the element; `InFrame` clears it when they stop.
    live: bool,
    /// The visible part of the element at its last paint, in window coordinates.
    rect: Option<Bounds<Pixels>>,
    shown: bool,
    /// False for a Mono page that is leaving. WebView2 can't pass clicks
    /// through to the page arriving under it, so it hides instead.
    interactive: bool,
}
#[cfg(any(target_os = "macos", windows))]
impl Hosted {
    /// Changes the native view's visibility only when it differs: hiding
    /// WebView2 and showing it again within one frame would flash.
    fn set_shown(&mut self, shown: bool) {
        if self.shown == shown {
            return;
        }
        self.shown = shown;
        #[cfg(target_os = "macos")]
        self.clip.setHidden(!shown);
        #[cfg(windows)]
        self.web.set_visible(shown);
    }
    fn hide(&mut self) {
        self.live = false;
        self.set_shown(false);
    }
}
/// Lives in GPUI's element state while frames draw the page's element, fresh
/// or replayed from a cached view. GPUI drops it after the first frame
/// without the element, which hides the native view. A page that moves in
/// the tree briefly has two, so the count decides.
#[cfg(any(target_os = "macos", windows))]
struct InFrame {
    host: Weak<RefCell<Option<Hosted>>>,
    count: Rc<std::cell::Cell<usize>>,
}
#[cfg(any(target_os = "macos", windows))]
impl Drop for InFrame {
    fn drop(&mut self) {
        self.count.set(self.count.get() - 1);
        if self.count.get() == 0
            && let Some(host) = self.host.upgrade()
            && let Ok(mut host) = host.try_borrow_mut()
            && let Some(hosted) = host.as_mut()
        {
            hosted.hide();
        }
    }
}
#[cfg(any(target_os = "macos", windows))]
fn keep_in_frame(
    window: &mut Window,
    host: &Host,
    count: &Rc<std::cell::Cell<usize>>,
    id: gpui::EntityId,
) {
    window.with_global_id(gpui::ElementId::View(id), |id, window| {
        window.with_element_state::<InFrame, _>(id, |state, _| {
            let state = state.unwrap_or_else(|| {
                count.set(count.get() + 1);
                InFrame {
                    host: Rc::downgrade(host),
                    count: count.clone(),
                }
            });
            ((), state)
        })
    });
}
#[cfg(any(target_os = "macos", windows))]
type Host = Rc<RefCell<Option<Hosted>>>;
/// The area of a popover or modal scrim while frames draw it.
#[cfg(any(target_os = "macos", windows))]
struct Cover {
    window: gpui::AnyWindowHandle,
    bounds: std::cell::Cell<Bounds<Pixels>>,
    app: gpui::AsyncApp,
}
/// Lives in GPUI's element state like `InFrame`, for a cover.
#[cfg(any(target_os = "macos", windows))]
struct CoverGuard(Rc<Cover>);
#[cfg(any(target_os = "macos", windows))]
impl Drop for CoverGuard {
    fn drop(&mut self) {
        // GPUI drops this after the frame's end, in no fixed order with the
        // `InFrame` of a page that left in the same frame. The frame that
        // closed the popover may also be the last for a while, so one more
        // frame decides what shows.
        let window = self.0.window;
        self.0
            .app
            .spawn(async move |cx| {
                cx.update_window(window, |root, _, cx| cx.notify(root.entity_id()))
            })
            .detach();
    }
}
#[cfg(any(target_os = "macos", windows))]
thread_local! {
    /// Every native view, so a frame's end can show or hide it.
    static HOSTS: RefCell<Vec<Weak<RefCell<Option<Hosted>>>>> = const { RefCell::new(Vec::new()) };
    static COVERS: RefCell<Vec<Weak<Cover>>> = const { RefCell::new(Vec::new()) };
}
#[cfg(any(target_os = "macos", windows))]
fn covered(window: gpui::AnyWindowHandle, bounds: Bounds<Pixels>) -> bool {
    COVERS.with_borrow(|covers| {
        covers.iter().filter_map(Weak::upgrade).any(|cover| {
            let area = cover.bounds.get();
            // An empty toast stack is a zero-height cover.
            cover.window == window && !area.is_empty() && area.intersects(&bounds)
        })
    })
}
/// Shows the window's web views that frames draw, unless a cover is over them.
#[cfg(any(target_os = "macos", windows))]
fn show_uncovered(window: gpui::AnyWindowHandle) {
    HOSTS.with_borrow(|hosts| {
        for host in hosts.iter().filter_map(Weak::upgrade) {
            if let Ok(mut host) = host.try_borrow_mut()
                && let Some(hosted) = host.as_mut()
                && hosted.window == window
                && hosted.live
            {
                let shown = hosted.rect.is_some_and(|rect| !covered(window, rect));
                #[cfg(windows)]
                let shown = shown && hosted.interactive;
                hosted.set_shown(shown);
            }
        }
    });
}
/// An invisible element that keeps the web views under its parent hidden
/// while frames draw it. Popovers and modal scrims carry one.
pub(crate) fn cover() -> gpui::AnyElement {
    #[cfg(any(target_os = "macos", windows))]
    {
        canvas(
            |bounds, window, cx| {
                window.with_global_id("html-cover".into(), |id, window| {
                    window.with_element_state::<CoverGuard, _>(id, |guard, window| {
                        let guard = guard.unwrap_or_else(|| {
                            let cover = Rc::new(Cover {
                                window: window.window_handle(),
                                bounds: std::cell::Cell::new(bounds),
                                app: cx.to_async(),
                            });
                            COVERS.with_borrow_mut(|covers| covers.push(Rc::downgrade(&cover)));
                            CoverGuard(cover)
                        });
                        guard.0.bounds.set(bounds);
                        ((), guard)
                    })
                })
            },
            |_, _, _, _| {},
        )
        .absolute()
        .inset_0()
        .into_any_element()
    }
    #[cfg(not(any(target_os = "macos", windows)))]
    gpui::div().into_any_element()
}
/// The last element a frame paints: shows the web views it drew. A cached
/// view replays its paint without running it, so the page's own paint can't
/// decide this.
pub(crate) fn end_frame() -> gpui::AnyElement {
    #[cfg(any(target_os = "macos", windows))]
    {
        gpui::deferred(canvas(
            |_, _, _| {},
            |_, _, window, _| {
                HOSTS.with_borrow_mut(|hosts| hosts.retain(|host| host.strong_count() > 0));
                COVERS.with_borrow_mut(|covers| covers.retain(|cover| cover.strong_count() > 0));
                show_uncovered(window.window_handle());
            },
        ))
        .with_priority(usize::MAX)
        .into_any_element()
    }
    #[cfg(not(any(target_os = "macos", windows)))]
    gpui::div().into_any_element()
}
enum Source {
    Markup(String),
    File(PathBuf),
}
#[cfg(any(target_os = "macos", windows))]
struct Markup {
    ink: gpui::Hsla,
    accent: gpui::Hsla,
    html: Arc<str>,
}
pub(crate) struct Html {
    source: Source,
    reloads: u32,
    interactive: bool,
    #[cfg(any(target_os = "macos", windows))]
    markup: Option<Markup>,
    #[cfg(any(target_os = "macos", windows))]
    view: Host,
    /// How many `InFrame` guards GPUI holds for this page.
    #[cfg(any(target_os = "macos", windows))]
    frames: Rc<std::cell::Cell<usize>>,
    /// The project a file page belongs to, which macOS lets it read.
    #[cfg(target_os = "macos")]
    root: PathBuf,
    /// WebView2 is starting.
    #[cfg(windows)]
    building: Rc<std::cell::Cell<bool>>,
    /// WebView2 couldn't start, such as when its runtime is missing.
    #[cfg(windows)]
    failed: Rc<std::cell::Cell<bool>>,
}
// Only the native web views show the sandbox; its test runs everywhere.
#[cfg(any(target_os = "macos", windows, test))]
fn escape(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('"', "&quot;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}
#[cfg(any(target_os = "macos", windows, test))]
fn sandbox(body: &str) -> String {
    let color = |value: gpui::Hsla| {
        let value: gpui::Rgba = value.into();
        format!(
            "#{:02x}{:02x}{:02x}",
            (value.r * 255.) as u8,
            (value.g * 255.) as u8,
            (value.b * 255.) as u8
        )
    };
    let ink: gpui::Rgba = crate::ui::theme::content(1.).into();
    let foreground = color(crate::ui::theme::content(1.));
    let accent = color(crate::ui::theme::accent_color());
    let light = ink.r * 0.2126 + ink.g * 0.7152 + ink.b * 0.0722 < 0.5;
    let background = if light { "#fafafa" } else { "#141414" };
    let muted = if light { "#737373" } else { "#969696" };
    let surface = if light { "#eeeeee" } else { "#222222" };
    let border = if light { "#d4d4d4" } else { "#383838" };
    let source = format!(
        "<!doctype html><meta http-equiv=\"Content-Security-Policy\" content=\"default-src 'none'; script-src 'unsafe-inline' https:; style-src 'unsafe-inline' https:; img-src data: blob: https:; font-src data: https:; media-src data: blob: https:; worker-src blob:; connect-src 'none'; frame-src 'none'; base-uri 'none'; form-action 'none'\"><meta name=viewport content=\"width=device-width,initial-scale=1\"><style>:root{{--background:{background};--foreground:{foreground};--muted:{muted};--surface:{surface};--border:{border};--accent:{accent};--link:{accent};--positive:#10b981;--negative:#f43f5e;--chart-1:{accent};--chart-2:#10b981;--chart-3:#f59e0b;--chart-4:#f43f5e;--chart-5:#a78bfa;--chart-6:#22d3ee;--font-sans:system-ui;--font-mono:ui-monospace,monospace;--radius:10px;color-scheme:{scheme}}}body{{font:14px var(--font-sans);margin:20px;color:var(--foreground);background:var(--background)}}a{{color:var(--link)}}</style>{body}",
        scheme = if light { "light" } else { "dark" }
    );
    format!(
        "<!doctype html><style>html,body,iframe{{margin:0;border:0;width:100%;height:100%;overflow:hidden;background:{background}}}</style><iframe sandbox=\"allow-scripts\" referrerpolicy=\"no-referrer\" srcdoc=\"{}\"></iframe>",
        escape(&source)
    )
}
/// The URL WebView2 loads a file from:
/// `C:\My Pages\é.html` -> `file:///C:/My%20Pages/%C3%A9.html`.
#[cfg(any(windows, test))]
fn file_url(path: &Path) -> String {
    let path = path.to_string_lossy().replace('\\', "/");
    // Canonical Windows paths carry the verbatim prefix.
    let path = match path.strip_prefix("//?/UNC/") {
        Some(share) => format!("//{share}"),
        None => path.strip_prefix("//?/").unwrap_or(&path).to_owned(),
    };
    let mut url = String::from(if path.starts_with("//") {
        "file:"
    } else {
        "file:///"
    });
    for byte in path.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' | b'/' | b':' => {
                url.push(byte as char)
            }
            _ => url.push_str(&format!("%{byte:02X}")),
        }
    }
    url
}
/// Whether `url` is `page` itself, whatever its fragment, query or case:
/// `file:///C:/a.html#top` is `file:///c:/A.html`.
#[cfg(any(windows, test))]
fn same_page(url: &str, page: &str) -> bool {
    fn base(url: &str) -> &str {
        url.split(['#', '?']).next().unwrap_or_default()
    }
    base(url).eq_ignore_ascii_case(base(page))
}
/// Whether `url` is a file on another machine than `page`'s, which Windows
/// would reach over SMB: `file://server/share/a.png` from `file:///C:/a.html`.
#[cfg(any(windows, test))]
fn foreign_file(url: &str, page: &str) -> bool {
    fn host(url: &str) -> Option<&str> {
        let rest = url
            .get(7..)
            .filter(|_| url[..7].eq_ignore_ascii_case("file://"))?;
        rest.split('/').next()
    }
    host(url).is_some_and(|host_name| {
        !host_name.is_empty() && !host(page).is_some_and(|own| own.eq_ignore_ascii_case(host_name))
    })
}
/// Opens an HTML file in the default browser. Windows would otherwise give
/// it to the `.html` type's app, which may be an editor.
pub(crate) fn open_in_browser(path: &Path) -> Result<(), String> {
    #[cfg(windows)]
    if webview2::open_in_browser(&file_url(path)).is_ok() {
        return Ok(());
    }
    monocode_core::files::open_path_with_default_app(path)
}
/// The folder a file page may read on macOS: the project `root` when the
/// page is inside it, otherwise its own folder.
#[cfg(any(target_os = "macos", test))]
fn read_access(path: &Path, root: &Path) -> PathBuf {
    if !root.as_os_str().is_empty() && path.starts_with(root) {
        root.to_path_buf()
    } else {
        path.parent().unwrap_or(path).to_path_buf()
    }
}
#[cfg(target_os = "macos")]
fn load(web: &WKWebView, page: &Page, root: &Path) {
    match page {
        Page::Markup(html) => unsafe {
            web.loadHTMLString_baseURL(&NSString::from_str(html), None);
        },
        Page::File(path, _) => {
            let url = NSURL::fileURLWithPath(&NSString::from_str(&path.to_string_lossy()));
            let folder = NSURL::fileURLWithPath_isDirectory(
                &NSString::from_str(&read_access(path, root).to_string_lossy()),
                true,
            );
            unsafe { web.loadFileURL_allowingReadAccessToURL(&url, &folder) };
        }
    }
}
impl Html {
    /// Generated markup, shown inside the opaque sandbox.
    pub(crate) fn new(body: String) -> Self {
        Self::from_source(Source::Markup(body))
    }
    /// An HTML file, loaded from its folder with scripts allowed. On macOS
    /// it may read the project `root` when inside it, otherwise its folder.
    pub(crate) fn file(path: PathBuf, root: &Path) -> Self {
        #[cfg_attr(not(target_os = "macos"), expect(unused_mut))]
        let mut html = Self::from_source(Source::File(path));
        #[cfg(target_os = "macos")]
        {
            html.root = root.to_path_buf();
        }
        #[cfg(not(target_os = "macos"))]
        let _ = root;
        html
    }
    fn from_source(source: Source) -> Self {
        #[cfg(any(target_os = "macos", windows))]
        let view: Host = Rc::new(RefCell::new(None));
        #[cfg(any(target_os = "macos", windows))]
        HOSTS.with_borrow_mut(|hosts| hosts.push(Rc::downgrade(&view)));
        Self {
            source,
            reloads: 0,
            interactive: true,
            #[cfg(any(target_os = "macos", windows))]
            markup: None,
            #[cfg(any(target_os = "macos", windows))]
            view,
            #[cfg(any(target_os = "macos", windows))]
            frames: Rc::default(),
            #[cfg(target_os = "macos")]
            root: PathBuf::new(),
            #[cfg(windows)]
            building: Rc::default(),
            #[cfg(windows)]
            failed: Rc::default(),
        }
    }
    #[cfg(any(target_os = "macos", windows))]
    fn page(&mut self) -> Page {
        let body = match &self.source {
            Source::File(path) => return Page::File(path.clone(), self.reloads),
            Source::Markup(body) => body,
        };
        let ink = crate::ui::theme::content(1.);
        let accent = crate::ui::theme::accent_color();
        if self
            .markup
            .as_ref()
            .is_none_or(|m| m.ink != ink || m.accent != accent)
        {
            self.markup = Some(Markup {
                ink,
                accent,
                html: sandbox(body).into(),
            });
        }
        Page::Markup(self.markup.as_ref().unwrap().html.clone())
    }
    /// Loads the page again; a file is read from disk.
    pub(crate) fn reload(&mut self, cx: &mut Context<Self>) {
        self.reloads += 1;
        cx.notify();
    }
    #[cfg(test)]
    pub(crate) fn reloads(&self) -> u32 {
        self.reloads
    }
    pub(crate) fn set_interactive(&mut self, interactive: bool) {
        if self.interactive != interactive {
            self.interactive = interactive;
        }
    }
    pub(crate) fn hide(&self) {
        #[cfg(any(target_os = "macos", windows))]
        if let Some(view) = self.view.borrow_mut().as_mut() {
            view.hide();
        }
    }
}
impl gpui::Render for Html {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        #[cfg(target_os = "macos")]
        {
            let host = self.view.clone();
            let frames = self.frames.clone();
            let entity = cx.entity_id();
            let page = self.page();
            let root = self.root.clone();
            let app = cx.to_async();
            let interactive = self.interactive;
            canvas(
                |_, _, _| {},
                move |bounds, _, window, _| {
                    keep_in_frame(window, &host, &frames, entity);
                    let Ok(handle) = HasWindowHandle::window_handle(window) else {
                        return;
                    };
                    let RawWindowHandle::AppKit(handle) = handle.as_raw() else {
                        return;
                    };
                    // GPUI keeps its AppKit content view alive throughout paint.
                    let parent = unsafe { &*handle.ns_view.as_ptr().cast::<NSView>() };
                    // AppKit frames are in window points; GPUI's are divided
                    // by the interface zoom.
                    let zoom = window.content_zoom();
                    let visible = bounds.intersect(&window.content_mask().bounds);
                    let clipped = visible.map(|v| v * zoom);
                    let bounds = bounds.map(|v| v * zoom);
                    let mut view = host.borrow_mut();
                    if visible.is_empty() {
                        if let Some(view) = view.as_mut() {
                            view.live = true;
                            view.rect = None;
                        }
                        return;
                    }
                    let x = f64::from(clipped.origin.x);
                    let y = f64::from(clipped.origin.y);
                    let clip_width = f64::from(clipped.size.width);
                    let clip_height = f64::from(clipped.size.height);
                    let width = f64::from(bounds.size.width);
                    let height = f64::from(bounds.size.height);
                    let y = if parent.isFlipped() {
                        y
                    } else {
                        parent.bounds().size.height - y - clip_height
                    };
                    let rect =
                        NSRect::new(NSPoint::new(x, y), NSSize::new(clip_width, clip_height));
                    // Keep the document at its full layout size while its
                    // native host follows GPUI's moving content mask.
                    let web_rect = NSRect::new(
                        NSPoint::new(
                            f64::from(bounds.left() - clipped.left()),
                            f64::from(bounds.top() - clipped.top()),
                        ),
                        NSSize::new(width, height),
                    );
                    if view.is_none() {
                        let Some(mtm) = MainThreadMarker::new() else {
                            return;
                        };
                        // No message handlers, app credentials or persistent
                        // website data are installed. Only a file page gets a
                        // file base URL.
                        let hosted = unsafe {
                            let clip = ArtifactClip::alloc(mtm)
                                .set_ivars(std::cell::Cell::new(interactive));
                            let clip: Retained<ArtifactClip> =
                                msg_send![super(clip), initWithFrame: rect];
                            clip.setWantsLayer(true);
                            clip.setClipsToBounds(true);
                            // The frame's end shows it.
                            clip.setHidden(true);
                            let config = WKWebViewConfiguration::new(mtm);
                            config
                                .preferences()
                                .setJavaScriptCanOpenWindowsAutomatically(false);
                            config.setWebsiteDataStore(
                                &WKWebsiteDataStore::nonPersistentDataStore(mtm),
                            );
                            let web = WKWebView::initWithFrame_configuration(
                                WKWebView::alloc(mtm),
                                web_rect,
                                &config,
                            );
                            let file = matches!(page, Page::File(..));
                            let guard = NavigationGuard::alloc(mtm).set_ivars((app.clone(), file));
                            let guard: Retained<NavigationGuard> = msg_send![super(guard), init];
                            web.setNavigationDelegate(Some(ProtocolObject::from_ref(&*guard)));
                            load(&web, &page, &root);
                            clip.addSubview(&web);
                            Hosted {
                                window: window.window_handle(),
                                clip,
                                web,
                                _guard: guard,
                                page: page.clone(),
                                live: true,
                                rect: None,
                                shown: false,
                            }
                        };
                        *view = Some(hosted);
                    }
                    if let Some(view) = view.as_mut() {
                        if !view.page.same(&page) {
                            load(&view.web, &page, &root);
                            view.page = page.clone();
                        }
                        parent.addSubview_positioned_relativeTo(
                            &view.clip,
                            NSWindowOrderingMode::Above,
                            None,
                        );
                        view.clip.ivars().set(interactive);
                        view.clip.setFrame(rect);
                        view.web.setFrame(web_rect);
                        unsafe { view.web.setPageZoom(f64::from(zoom)) };
                        view.live = true;
                        view.rect = Some(visible);
                    }
                },
            )
            .size_full()
            .into_any_element()
        }
        #[cfg(windows)]
        {
            if self.failed.get() {
                return div()
                    .p_4()
                    .text_color(crate::ui::theme::content(0.6))
                    .child("The page can’t be shown here: Microsoft Edge WebView2 didn’t start. Open it in your browser instead.")
                    .into_any_element();
            }
            let host = self.view.clone();
            let frames = self.frames.clone();
            let entity = cx.entity_id();
            let building = self.building.clone();
            let failed = self.failed.clone();
            let page = self.page();
            let app = cx.to_async();
            let interactive = self.interactive;
            canvas(
                |_, _, _| {},
                move |bounds, _, window, _| {
                    keep_in_frame(window, &host, &frames, entity);
                    let visible = bounds.intersect(&window.content_mask().bounds);
                    // ponytail: WebView2 is sized to the visible part, so a
                    // page reflows while its pane slides in. A clip region
                    // on the child window would keep its layout, as macOS does.
                    let scale = window.scale_factor();
                    let physical = |v: Pixels| (f32::from(v) * scale).round();
                    let rect = wry::Rect {
                        position: wry::dpi::PhysicalPosition::new(
                            physical(visible.origin.x) as i32,
                            physical(visible.origin.y) as i32,
                        )
                        .into(),
                        size: wry::dpi::PhysicalSize::new(
                            physical(visible.size.width) as u32,
                            physical(visible.size.height) as u32,
                        )
                        .into(),
                    };
                    let mut view = host.borrow_mut();
                    if view.is_none() {
                        if !building.get()
                            && let Ok(handle) =
                                wry::raw_window_handle::HasWindowHandle::window_handle(&*window)
                        {
                            building.set(true);
                            let parent = webview2::Parent(handle.as_raw());
                            let host = Rc::downgrade(&host);
                            let id = window.window_handle();
                            let (page, building, failed) =
                                (page.clone(), building.clone(), failed.clone());
                            app.spawn(async move |cx| {
                                let web = webview2::Web::build(&parent, &page, rect, cx.clone());
                                building.set(false);
                                match web {
                                    Ok(web) => {
                                        if let Some(host) = host.upgrade() {
                                            *host.borrow_mut() = Some(Hosted {
                                                window: id,
                                                web,
                                                page,
                                                bounds: Some(rect),
                                                zoom: 1.,
                                                live: false,
                                                rect: None,
                                                shown: false,
                                                interactive: true,
                                            });
                                        }
                                    }
                                    Err(error) => {
                                        log::warn!("HTML preview: WebView2 didn't start: {error}");
                                        failed.set(true);
                                    }
                                }
                                // Paints the page's element again, even in a
                                // cached view, so the frame's end shows it.
                                let _ = cx.update_window(id, |_, window, _| window.refresh());
                            })
                            .detach();
                        }
                        return;
                    }
                    let Some(view) = view.as_mut() else {
                        return;
                    };
                    view.live = true;
                    view.interactive = interactive;
                    view.rect = (!visible.is_empty()).then_some(visible);
                    if visible.is_empty() {
                        return;
                    }
                    if !view.page.same(&page) {
                        view.web.load(&page);
                        view.page = page.clone();
                    }
                    if view.bounds != Some(rect) {
                        let _ = view.web.view.set_bounds(rect);
                        view.bounds = Some(rect);
                    }
                    // WebView2 follows the display's scale itself; the zoom
                    // matches MCode's interface zoom.
                    let zoom = window.content_zoom();
                    if view.zoom != zoom {
                        let _ = view.web.view.zoom(f64::from(zoom));
                        view.zoom = zoom;
                    }
                },
            )
            .size_full()
            .into_any_element()
        }
        #[cfg(not(any(target_os = "macos", windows)))]
        {
            let _ = cx;
            match &self.source {
                Source::Markup(body) => div().p_4().child(body.clone()).into_any_element(),
                Source::File(path) => div()
                    .p_4()
                    .child(path.display().to_string())
                    .into_any_element(),
            }
        }
    }
}
#[cfg(any(target_os = "macos", windows))]
impl Drop for Html {
    fn drop(&mut self) {
        let view = self.view.borrow_mut().take();
        #[cfg(target_os = "macos")]
        if let Some(view) = view {
            view.clip.removeFromSuperview();
        }
        // Dropping wry's view destroys its child window, and returns its
        // keyboard focus to the GPUI window.
        #[cfg(windows)]
        drop(view);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn generated_markup_cannot_escape_the_opaque_iframe() {
        let html = sandbox("\"><script>window.top.postMessage('x','*')</script>");
        assert!(html.contains("sandbox=\"allow-scripts\""));
        assert!(!html.contains("allow-same-origin"));
        assert!(html.contains("connect-src 'none'"));
        assert!(html.contains("&lt;script&gt;"));
    }
    #[test]
    fn file_urls_escape_spaces_and_non_ascii_names() {
        let url = |path: &str| file_url(std::path::Path::new(path));
        assert_eq!(
            url(r"C:\Users\Tiar Nur\Documents\ai growth é#1.html"),
            "file:///C:/Users/Tiar%20Nur/Documents/ai%20growth%20%C3%A9%231.html"
        );
        assert_eq!(url(r"\\?\C:\a\b.html"), "file:///C:/a/b.html");
        assert_eq!(url(r"\\server\share\b.html"), "file://server/share/b.html");
        assert_eq!(
            url(r"\\?\UNC\server\share\b.html"),
            "file://server/share/b.html"
        );
    }
    #[test]
    fn pages_stay_on_themselves_and_off_file_shares() {
        let page = "file:///C:/My%20Pages/a.html";
        assert!(same_page("file:///c:/my%20pages/A.html#top", page));
        assert!(same_page("file:///C:/My%20Pages/a.html?x=1", page));
        assert!(!same_page("file:///C:/My%20Pages/b.html", page));
        assert!(!same_page("https://example.com/", page));
        assert!(foreign_file("file://attacker/share/a.png", page));
        assert!(foreign_file(
            "FILE://attacker/a.png",
            "http://mcode-page.localhost/"
        ));
        assert!(!foreign_file("file:///C:/My%20Pages/style.css", page));
        assert!(!foreign_file("https://example.com/a.png", page));
        // A page on a share still loads its own files.
        assert!(!foreign_file(
            "file://server/s/a.css",
            "file://server/s/a.html"
        ));
    }
    #[test]
    fn a_file_reads_its_project_or_else_its_folder() {
        let folder = |path: &str, root: &str| read_access(Path::new(path), Path::new(root));
        assert_eq!(
            folder("/work/app/docs/a.html", "/work/app"),
            Path::new("/work/app")
        );
        assert_eq!(
            folder("/tmp/out/a.html", "/work/app"),
            Path::new("/tmp/out")
        );
        assert_eq!(folder("/tmp/out/a.html", ""), Path::new("/tmp/out"));
    }
}
