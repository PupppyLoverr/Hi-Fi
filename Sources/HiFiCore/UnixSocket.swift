import Foundation
#if canImport(Darwin)
import Darwin
#endif

public enum SocketError: Error, CustomStringConvertible {
    case create(String)
    case bind(String)
    case listen(String)
    case connect(String)
    case io(String)
    case timeout

    public var description: String {
        switch self {
        case .create(let m): return "socket: \(m)"
        case .bind(let m):   return "bind: \(m)"
        case .listen(let m): return "listen: \(m)"
        case .connect(let m): return "connect: \(m)"
        case .io(let m):     return "io: \(m)"
        case .timeout:       return "timeout"
        }
    }
}

func unixSockaddr(path: String) -> (sockaddr_un, socklen_t) {
    var addr = sockaddr_un()
    addr.sun_family = sa_family_t(AF_UNIX)
    let maxLen = MemoryLayout.size(ofValue: addr.sun_path)
    _ = withUnsafeMutablePointer(to: &addr.sun_path) { ptr in
        ptr.withMemoryRebound(to: CChar.self, capacity: maxLen) { cstr in
            strncpy(cstr, path, maxLen - 1)
        }
    }
    return (addr, socklen_t(MemoryLayout<sockaddr_un>.size))
}

/// Line-delimited JSON UNIX-socket server. Handler runs on a background queue;
/// it receives one request line and returns one response line.
public final class UnixSocketServer: @unchecked Sendable {
    private let path: String
    private var listenFD: Int32 = -1
    private var acceptSource: DispatchSourceRead?
    private let queue = DispatchQueue(label: "hifi.ipc", attributes: .concurrent)
    public var onRequest: (Data) -> Data = { _ in Data("{\"ok\":false,\"error\":\"no handler\"}\n".utf8) }

    public init(path: String) { self.path = path }

    public func start() throws {
        unlink(path)
        try? FileManager.default.createDirectory(
            atPath: (path as NSString).deletingLastPathComponent, withIntermediateDirectories: true)
        listenFD = socket(AF_UNIX, SOCK_STREAM, 0)
        guard listenFD >= 0 else { throw SocketError.create(String(cString: strerror(errno))) }
        var flags = fcntl(listenFD, F_GETFL)
        _ = fcntl(listenFD, F_SETFL, flags | O_NONBLOCK)

        var (addr, len) = unixSockaddr(path: path)
        let bound = withUnsafePointer(to: &addr) { p in
            p.withMemoryRebound(to: sockaddr.self, capacity: 1) { Darwin.bind(listenFD, $0, len) }
        }
        guard bound == 0 else { throw SocketError.bind(String(cString: strerror(errno))) }
        guard Darwin.listen(listenFD, 32) == 0 else {
            throw SocketError.listen(String(cString: strerror(errno)))
        }

        let src = DispatchSource.makeReadSource(fileDescriptor: listenFD, queue: queue)
        src.setEventHandler { [weak self] in self?.acceptLoop() }
        src.setCancelHandler { [weak self] in
            if let fd = self?.listenFD, fd >= 0 { close(fd) }
        }
        acceptSource = src
        src.resume()
    }

    private func acceptLoop() {
        while true {
            var caddr = sockaddr()
            var clen = socklen_t(MemoryLayout<sockaddr>.size)
            let cfd = withUnsafeMutablePointer(to: &caddr) { p in
                Darwin.accept(listenFD, p, &clen)
            }
            if cfd < 0 { break }
            queue.async { [weak self] in self?.serve(fd: cfd) }
        }
    }

    private func serve(fd: Int32) {
        defer { close(fd) }
        var buf = Data()
        var chunk = [UInt8](repeating: 0, count: 8192)
        readLoop: while true {
            let n = chunk.withUnsafeMutableBytes { ptr in
                Darwin.read(fd, ptr.baseAddress, ptr.count)
            }
            if n <= 0 { break }
            buf.append(contentsOf: chunk[0..<n])
            if let nl = buf.firstIndex(of: 0x0A) {
                let line = buf[buf.startIndex..<nl]
                let reply = onRequest(Data(line))
                reply.withUnsafeBytes { ptr in
                    _ = Darwin.write(fd, ptr.baseAddress, ptr.count)
                }
                buf.removeSubrange(buf.startIndex...nl)
            }
        }
    }

    public func stop() { acceptSource?.cancel() }
    deinit { stop() }
}

/// Blocking client used by the `hifi` CLI.
public enum UnixSocketClient {
    public static func request(path: String, payload: Data, timeout: TimeInterval = 15) throws -> Data {
        let fd = socket(AF_UNIX, SOCK_STREAM, 0)
        guard fd >= 0 else { throw SocketError.create(String(cString: strerror(errno))) }
        defer { close(fd) }

        var (addr, len) = unixSockaddr(path: path)
        let rc = withUnsafePointer(to: &addr) { p in
            p.withMemoryRebound(to: sockaddr.self, capacity: 1) { Darwin.connect(fd, $0, len) }
        }
        guard rc == 0 else { throw SocketError.connect(String(cString: strerror(errno))) }

        // read timeout
        var tv = timeval(tv_sec: Int(timeout), tv_usec: 0)
        setsockopt(fd, SOL_SOCKET, SO_RCVTIMEO, &tv, socklen_t(MemoryLayout<timeval>.size))

        try payload.withUnsafeBytes { ptr in
            if Darwin.write(fd, ptr.baseAddress, ptr.count) < 0 {
                throw SocketError.io(String(cString: strerror(errno)))
            }
        }

        var out = Data()
        var chunk = [UInt8](repeating: 0, count: 8192)
        while true {
            let n = chunk.withUnsafeMutableBytes { ptr in
                Darwin.read(fd, ptr.baseAddress, ptr.count)
            }
            if n < 0 {
                if errno == EAGAIN { throw SocketError.timeout }
                throw SocketError.io(String(cString: strerror(errno)))
            }
            if n == 0 { break }
            out.append(contentsOf: chunk[0..<n])
            if out.contains(0x0A) { break }
        }
        return out
    }

    public static func ping(path: String) -> Bool {
        guard let req = try? IPCRequest(command: IPCCommand.ping).encode(),
              let data = try? request(path: path, payload: req, timeout: 2),
              let resp = try? JSONDecoder().decode(IPCResponse.self, from: data)
        else { return false }
        return resp.ok
    }
}
