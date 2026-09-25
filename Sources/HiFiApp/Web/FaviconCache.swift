import Cocoa

/// Tiny favicon fetcher + memory cache. Fetches <origin>/favicon.ico once per
/// host, keeps the bitmap in RAM only.
@MainActor
final class FaviconCache {
    static let shared = FaviconCache()

    private var images: [String: NSImage] = [:]
    private var pending: Set<String> = []

    /// Returns the cached icon for `url`'s host, or nil; kicks off a fetch on
    /// miss. Sidebar rows re-render on the next state publish.
    func image(for url: String) -> NSImage? {
        guard let host = URL(string: url)?.host?.lowercased(), !host.isEmpty else { return nil }
        if let img = images[host] { return img }
        guard !pending.contains(host) else { return nil }
        pending.insert(host)
        Task.detached { [weak self] in
            defer { Task { @MainActor in self?.pending.remove(host) } }
            for path in ["/favicon.ico", "/favicon.png", "/apple-touch-icon.png"] {
                guard let u = URL(string: "https://\(host)\(path)") else { continue }
                if let (data, resp) = try? await URLSession.shared.data(from: u),
                   (resp as? HTTPURLResponse)?.statusCode == 200,
                   let img = NSImage(data: data), img.size.width > 0 {
                    await MainActor.run { self?.images[host] = img }
                    return
                }
            }
        }
        return nil
    }
}
