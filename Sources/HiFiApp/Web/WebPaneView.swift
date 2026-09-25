import Cocoa
import WebKit
import HiFiCore

/// WKWebView + hairline progress bar. One shared process pool; per-profile
/// data stores keyed by the space's bound Profile.
@MainActor
final class WebPaneView: PaneHostView {

    static let sharedProcessPool = WKProcessPool()

    let webView: WKWebView
    private let tabID: String
    private weak var store: BrowserStore?
    private let progressBar = NSView()
    private var progressWidth: NSLayoutConstraint?
    private var kvoToken: NSKeyValueObservation?
    private var titleToken: NSKeyValueObservation?
    private var urlToken: NSKeyValueObservation?

    override var firstResponderView: NSView { webView }

    init(tab: HiFiCore.Tab, store: BrowserStore) {
        self.tabID = tab.id
        self.store = store

        let config = WKWebViewConfiguration()
        config.processPool = Self.sharedProcessPool
        config.preferences.setValue(true, forKey: "developerExtrasEnabled")
        config.websiteDataStore = Self.dataStore(for: tab, store: store)
        // Safari-compatible UA — never claim Chrome/Blink.
        config.applicationNameForUserAgent = " Hi-Fi/0.1 (WebKit)"

        webView = WKWebView(frame: .zero, configuration: config)
        super.init(frame: .zero)
        wantsLayer = true

        webView.translatesAutoresizingMaskIntoConstraints = false
        addSubview(webView)
        progressBar.wantsLayer = true
        progressBar.layer?.backgroundColor = NSColor.controlAccentColor.cgColor
        progressBar.translatesAutoresizingMaskIntoConstraints = false
        progressBar.isHidden = true
        addSubview(progressBar)

        NSLayoutConstraint.activate([
            webView.leadingAnchor.constraint(equalTo: leadingAnchor),
            webView.trailingAnchor.constraint(equalTo: trailingAnchor),
            webView.topAnchor.constraint(equalTo: topAnchor),
            webView.bottomAnchor.constraint(equalTo: bottomAnchor),
            progressBar.topAnchor.constraint(equalTo: topAnchor),
            progressBar.leadingAnchor.constraint(equalTo: leadingAnchor),
            progressBar.heightAnchor.constraint(equalToConstant: 2),
        ])
        progressWidth = progressBar.widthAnchor.constraint(equalToConstant: 0)
        progressWidth?.isActive = true

        webView.navigationDelegate = self
        webView.uiDelegate = self

        kvoToken = webView.observe(\.estimatedProgress) { [weak self] wv, _ in
            self?.updateProgress(wv.estimatedProgress)
        }
        titleToken = webView.observe(\.title) { [weak self] wv, _ in
            if let t = wv.title, !t.isEmpty { self?.store?.updateTab(self?.tabID ?? "", title: t) }
        }
        urlToken = webView.observe(\.url) { [weak self] wv, _ in
            if let u = wv.url?.absoluteString { self?.store?.updateTab(self?.tabID ?? "", url: u) }
        }

        if let url = URL(string: tab.url), tab.url.hasPrefix("http") || tab.url.hasPrefix("file") {
            webView.load(URLRequest(url: url))
        } else if tab.kind == .preview, let path = tab.projectPath {
            // preview bound to a worktree: prefer localhost URL stored in tab.url,
            // else show the directory index / a hint page
            if let url = URL(string: tab.url) { webView.load(URLRequest(url: url)) }
            else { loadPlaceholder("Preview", detail: path) }
        } else {
            loadPlaceholder("Hi-Fi", detail: tab.url)
        }
    }

    required init?(coder: NSCoder) { fatalError() }

    deinit {
        kvoToken?.invalidate(); titleToken?.invalidate(); urlToken?.invalidate()
    }

    // MARK: helpers

    static func dataStore(for tab: HiFiCore.Tab, store: BrowserStore) -> WKWebsiteDataStore {
        // Space-bound profile -> persistent per-profile data store.
        if let (space, _) = store.groupOf(tabID: tab.id),
           let pid = space.profileID,
           let profile = store.state.profiles.first(where: { $0.id == pid }) {
            if #available(macOS 15.0, *) {
                return WKWebsiteDataStore(forIdentifier: profile.identifier)
            }
            return .default()
        }
        return .default()
    }

    private func loadPlaceholder(_ title: String, detail: String) {
        let html = """
        <html><body style="font-family:-apple-system;display:flex;height:100vh;
        align-items:center;justify-content:center;flex-direction:column;
        color:#888;background:transparent">
        <h1 style="font-weight:600">\(title)</h1><p>\(detail)</p></body></html>
        """
        webView.loadHTMLString(html, baseURL: nil)
    }

    private func updateProgress(_ p: Double) {
        progressBar.isHidden = p >= 1
        progressWidth?.constant = bounds.width * CGFloat(p)
    }

    override func layout() {
        super.layout()
        if !progressBar.isHidden {
            progressWidth?.constant = bounds.width * CGFloat(webView.estimatedProgress)
        }
    }

    // MARK: engine-facing

    func navigate(_ url: String) {
        let u = url.hasPrefix("http") || url.hasPrefix("file") ? url : "https://\(url)"
        if let real = URL(string: u) { webView.load(URLRequest(url: real)) }
    }
    func goBack() { webView.goBack() }
    func goForward() { webView.goForward() }
    func reload() { webView.reload() }
    func stopLoading() { webView.stopLoading() }

    func evalJS(_ js: String) async throws -> Any? {
        try await webView.evaluateJavaScript(js)
    }

    func snapshotPNG() async throws -> Data {
        let config = WKSnapshotConfiguration()
        let img = try await webView.takeSnapshot(configuration: config)
        guard let tiff = img.tiffRepresentation,
              let rep = NSBitmapImageRep(data: tiff),
              let png = rep.representation(using: .png, properties: [:])
        else { throw NSError(domain: "hifi", code: 1,
                             userInfo: [NSLocalizedDescriptionKey: "snapshot encode failed"]) }
        return png
    }

    func find(_ needle: String, backwards: Bool = false, wrap: Bool = true) {
        let js = "window.find(\(needle.asJSString), false, \(backwards), \(wrap), false, false, false)"
        webView.evaluateJavaScript(js) { _, _ in }
    }
}

extension String {
    var asJSString: String {
        let escaped = self.replacingOccurrences(of: "\\", with: "\\\\")
            .replacingOccurrences(of: "\"", with: "\\\"")
            .replacingOccurrences(of: "\n", with: "\\n")
        return "\"\(escaped)\""
    }
}

// MARK: - navigation delegate

extension WebPaneView: WKNavigationDelegate {
    func webView(_ webView: WKWebView, didCommit navigation: WKNavigation!) {
        guard let url = webView.url?.absoluteString else { return }
        store?.updateTab(tabID, url: url)
    }
    func webView(_ webView: WKWebView, didFinish navigation: WKNavigation!) {
        if let url = webView.url?.absoluteString {
            store?.recordHistory(url: url, title: webView.title ?? "")
        }
    }
    func webView(_ webView: WKWebView, didFail navigation: WKNavigation!, withError error: Error) {
        store?.toast("Load failed: \(error.localizedDescription)")
    }
    func webView(_ webView: WKWebView,
                 decidePolicyFor navigationResponse: WKNavigationResponse) async -> WKNavigationResponsePolicy {
        if navigationResponse.canShowMIMEType { return .allow }
        // undisplayable -> download
        return .download
    }
    func webView(_ webView: WKWebView,
                 navigationAction: WKNavigationAction,
                 didBecome download: WKDownload) {
        download.delegate = self
    }
    func webView(_ webView: WKWebView,
                 navigationResponse: WKNavigationResponse,
                 didBecome download: WKDownload) {
        download.delegate = self
    }
}

// MARK: - UI delegate: popups open as tabs, downloads into the tray

extension WebPaneView: WKUIDelegate {
    func webView(_ webView: WKWebView,
                 createWebViewWith configuration: WKWebViewConfiguration,
                 for navigationAction: WKNavigationAction,
                 windowFeatures: WKWindowFeatures) -> WKWebView? {
        if let url = navigationAction.request.url {
            _ = store?.openTab(url.absoluteString)
        }
        return nil
    }

    func webView(_ webView: WKWebView,
                 runJavaScriptAlertPanelWithMessage message: String,
                 initiatedByFrame frame: WKFrameInfo) async {
        let alert = NSAlert()
        alert.messageText = webView.url?.host ?? "Page"
        alert.informativeText = message
        alert.addButton(withTitle: "OK")
        alert.runModal()
    }

    func webView(_ webView: WKWebView,
                 runJavaScriptConfirmPanelWithMessage message: String,
                 initiatedByFrame frame: WKFrameInfo) async -> Bool {
        let alert = NSAlert()
        alert.messageText = webView.url?.host ?? "Confirm"
        alert.informativeText = message
        alert.addButton(withTitle: "OK")
        alert.addButton(withTitle: "Cancel")
        return alert.runModal() == .alertFirstButtonReturn
    }
}

extension WebPaneView: WKDownloadDelegate {
    public func download(_ download: WKDownload,
                         decideDestinationUsing response: URLResponse,
                         suggestedFilename: String) async -> URL? {
        let dest = FileManager.default.urls(for: .downloadsDirectory, in: .userDomainMask).first!
            .appendingPathComponent(suggestedFilename)
        store?.registerDownload(filename: suggestedFilename,
                                url: response.url?.absoluteString ?? "",
                                destination: dest.path)
        return dest
    }
    public func download(_ download: WKDownload, didFinishDownloadingTo location: URL) {
        store?.finishDownload(url: download.originalRequest?.url?.absoluteString ?? "")
        store?.toast("Downloaded \(location.lastPathComponent)")
    }
    public func download(_ download: WKDownload, didFailWithError error: Error,
                         resumeData: Data?) {
        store?.toast("Download failed: \(error.localizedDescription)")
    }
}
