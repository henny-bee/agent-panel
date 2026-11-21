//! Edge WebView2 through wry, for HTML previews on Windows. A page stays on
//! itself: it can't navigate away, open windows, download, or load from a
//! file share. Only a link the user clicks opens, in the browser.
use super::{Page, file_url, foreign_file, same_page};
use std::sync::{Arc, Mutex};
use webview2_com::{
    Microsoft::Web::WebView2::Win32::{
        COREWEBVIEW2_WEB_RESOURCE_CONTEXT, COREWEBVIEW2_WEB_RESOURCE_CONTEXT_ALL,
        COREWEBVIEW2_WEB_RESOURCE_CONTEXT_DOCUMENT, ICoreWebView2NavigationStartingEventArgs,
    },
    NavigationStartingEventHandler, NewWindowRequestedEventHandler,
    WebResourceRequestedEventHandler, take_pwstr,
};
use windows::{
    Win32::{
        Foundation::HWND,
        UI::{
            Input::KeyboardAndMouse::{GetFocus, SetFocus},
            Shell::{SEE_MASK_CLASSNAME, SHELLEXECUTEINFOW, ShellExecuteExW},
            WindowsAndMessaging::{IsChild, SW_SHOWNORMAL},
        },
    },
    core::{BOOL, HSTRING, PCWSTR, PWSTR, w},
};
use wry::{
    WebViewExtWindows,
    raw_window_handle::{HandleError, HasWindowHandle, RawWindowHandle, WindowHandle},
};

/// Where a Mono page's sandbox wrapper is served from, so it isn't bound by
/// `NavigateToString`'s 2 MB limit.
const MARKUP_URL: &str = "http://mcode-page.localhost/";

/// The GPUI window a web view attaches to, from the handle paint saw.
pub(super) struct Parent(pub(super) RawWindowHandle);
impl HasWindowHandle for Parent {
    fn window_handle(&self) -> Result<WindowHandle<'_>, HandleError> {
        // A closed window's handle fails to parent the view instead.
        Ok(unsafe { WindowHandle::borrow_raw(self.0) })
    }
}

/// A page's web view, and the markup its wrapper URL serves.
pub(super) struct Web {
    pub(super) view: wry::WebView,
    parent: HWND,
    markup: Arc<Mutex<Arc<str>>>,
}
impl Web {
    /// Creates WebView2 off the frame: it pumps the message loop until the
    /// view exists, and a frame can't be drawing then.
    pub(super) fn build(
        parent: &Parent,
        page: &Page,
        bounds: wry::Rect,
        app: gpui::AsyncApp,
    ) -> wry::Result<Self> {
        let RawWindowHandle::Win32(handle) = parent.0 else {
            return Err(wry::Error::UnsupportedWindowHandle);
        };
        let markup = Arc::new(Mutex::new(Arc::from("")));
        let url = page_url(page, &markup);
        let served = markup.clone();
        // WebView2 writes its profile beside the executable unless given a
        // folder, and an installed app can't write there. InPrivate keeps
        // nothing in it between runs.
        let mut context = wry::WebContext::new(Some(std::env::temp_dir().join("MCode WebView2")));
        let view = wry::WebViewBuilder::new_with_web_context(&mut context)
            .with_bounds(bounds)
            .with_visible(false)
            .with_focused(false)
            .with_incognito(true)
            .with_download_started_handler(|_, _| false)
            .with_custom_protocol("mcode-page".into(), move |_, _| {
                let body = served
                    .lock()
                    .map_or_else(|_| Vec::new(), |m| m.as_bytes().to_vec());
                wry::http::Response::builder()
                    .header("Content-Type", "text/html; charset=utf-8")
                    .body(body.into())
                    .unwrap_or_default()
            })
            .with_url(&url)
            .build_as_child(parent)?;
        contain(&view, url, matches!(page, Page::Markup(_)), app)
            .map_err(|error| wry::Error::WebView2Error(error.into()))?;
        Ok(Self {
            view,
            parent: HWND(handle.hwnd.get() as _),
            markup,
        })
    }

    pub(super) fn load(&self, page: &Page) {
        if let Err(error) = self.view.load_url(&page_url(page, &self.markup)) {
            log::warn!("HTML preview: the page didn't load: {error}");
        }
    }

    pub(super) fn set_visible(&self, shown: bool) {
        if !shown {
            release_focus(self.parent);
        }
        let _ = self.view.set_visible(shown);
    }
}
impl Drop for Web {
    fn drop(&mut self) {
        release_focus(self.parent);
    }
}

/// The URL that shows `page`; a Mono page's markup is served from it.
fn page_url(page: &Page, markup: &Mutex<Arc<str>>) -> String {
    match page {
        Page::Markup(html) => {
            if let Ok(mut markup) = markup.lock() {
                *markup = html.clone();
            }
            MARKUP_URL.to_owned()
        }
        Page::File(path, ..) => file_url(path),
    }
}

/// Keeps a page on itself, so script can't send it anywhere: navigating
/// elsewhere is cancelled, and a link the user clicked opens in the browser
/// instead. A Mono page's sandbox frame can't navigate at all, and nothing
/// loads from a file share.
fn contain(
    view: &wry::WebView,
    page: String,
    markup: bool,
    app: gpui::AsyncApp,
) -> windows::core::Result<()> {
    let core = view.webview();
    let env = view.environment();
    let mut token = 0;
    let (frames, popups) = (app.clone(), app.clone());
    let (top, frame_page) = (page.clone(), page.clone());
    unsafe {
        core.add_NavigationStarting(
            &NavigationStartingEventHandler::create(Box::new(move |_, args| {
                decide(args, &app, |url| same_page(url, &top))
            })),
            &mut token,
        )?;
        core.add_FrameNavigationStarting(
            &NavigationStartingEventHandler::create(Box::new(move |_, args| {
                decide(args, &frames, |url| {
                    if markup {
                        url.starts_with("about:")
                    } else {
                        !foreign_file(url, &frame_page)
                    }
                })
            })),
            &mut token,
        )?;
        // wry denies every new window; only a click opens one, in the browser.
        core.add_NewWindowRequested(
            &NewWindowRequestedEventHandler::create(Box::new(move |_, args| {
                let Some(args) = args else { return Ok(()) };
                let (mut uri, mut user) = (PWSTR::null(), BOOL(0));
                args.Uri(&mut uri)?;
                args.IsUserInitiated(&mut user)?;
                let uri = take_pwstr(uri);
                if user.as_bool() {
                    open_link(&popups, uri);
                }
                Ok(())
            })),
            &mut token,
        )?;
        core.AddWebResourceRequestedFilter(w!("file://*"), COREWEBVIEW2_WEB_RESOURCE_CONTEXT_ALL)?;
        if markup {
            // A cancelled navigation has already sent its request, so a Mono
            // page's documents other than its wrapper stop here instead.
            core.AddWebResourceRequestedFilter(
                w!("*"),
                COREWEBVIEW2_WEB_RESOURCE_CONTEXT_DOCUMENT,
            )?;
        }
        core.add_WebResourceRequested(
            &WebResourceRequestedEventHandler::create(Box::new(move |_, args| {
                let Some(args) = args else { return Ok(()) };
                let (mut uri, mut context) = (PWSTR::null(), COREWEBVIEW2_WEB_RESOURCE_CONTEXT(0));
                args.Request()?.Uri(&mut uri)?;
                args.ResourceContext(&mut context)?;
                let uri = take_pwstr(uri);
                let stray = markup
                    && context == COREWEBVIEW2_WEB_RESOURCE_CONTEXT_DOCUMENT
                    && !uri.starts_with(MARKUP_URL);
                if stray || foreign_file(&uri, &page) {
                    let denied =
                        env.CreateWebResourceResponse(None, 403, w!("Forbidden"), w!(""))?;
                    args.SetResponse(&denied)?;
                }
                Ok(())
            })),
            &mut token,
        )?;
    }
    Ok(())
}

/// Cancels a navigation `allowed` refuses, and opens it in the browser when
/// the user started it.
fn decide(
    args: Option<ICoreWebView2NavigationStartingEventArgs>,
    app: &gpui::AsyncApp,
    allowed: impl Fn(&str) -> bool,
) -> windows::core::Result<()> {
    let Some(args) = args else { return Ok(()) };
    let (mut uri, mut user) = (PWSTR::null(), BOOL(0));
    unsafe {
        args.Uri(&mut uri)?;
        args.IsUserInitiated(&mut user)?;
    }
    let uri = take_pwstr(uri);
    if !allowed(&uri) {
        unsafe { args.SetCancel(true)? };
        if user.as_bool() {
            open_link(app, uri);
        }
    }
    Ok(())
}

/// Opens a web link or a local file the user clicked, off the UI thread.
fn open_link(app: &gpui::AsyncApp, url: String) {
    if ["https://", "http://", "file:///"]
        .iter()
        .any(|scheme| url.starts_with(scheme))
    {
        app.background_executor()
            .spawn(async move {
                if let Err(error) = open_in_browser(&url) {
                    log::warn!("HTML preview: couldn't open {url}: {error}");
                }
            })
            .detach();
    }
}

/// Opens `url` with the default browser, even a `file:` URL that Windows
/// would give to the `.html` type's own app, such as an editor.
pub(super) fn open_in_browser(url: &str) -> Result<(), String> {
    let url = HSTRING::from(url);
    let mut info = SHELLEXECUTEINFOW {
        cbSize: size_of::<SHELLEXECUTEINFOW>() as u32,
        fMask: SEE_MASK_CLASSNAME,
        lpClass: w!("http"),
        lpFile: PCWSTR(url.as_ptr()),
        nShow: SW_SHOWNORMAL.0,
        ..Default::default()
    };
    unsafe { ShellExecuteExW(&mut info) }.map_err(|error| error.message())
}

/// Gives keyboard focus back to the GPUI window when the page holds it, so
/// typing reaches MCode once the page hides or closes.
fn release_focus(parent: HWND) {
    unsafe {
        let focus = GetFocus();
        if focus != parent && IsChild(parent, focus).as_bool() {
            let _ = SetFocus(Some(parent));
        }
    }
}
