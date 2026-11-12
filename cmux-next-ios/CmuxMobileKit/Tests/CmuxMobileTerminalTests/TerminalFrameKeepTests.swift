#if os(iOS)
import CmuxMobileArtifacts
import Foundation
import Testing
import UIKit
@testable import CmuxMobileTerminal

/// Leo, 2026-11-12, on LTE: "wack reconnection state / just show the cache".
///
/// No path may store a blank page, or one from before the tab's first
/// attach, over a tab's good cached frame: a tab cooled or closed before it
/// attached, the background save, the throttled saver and the placeholder
/// swap all take their frame through `TerminalFrameKeep`.
@MainActor
@Suite(.serialized) struct TerminalFrameKeepTests {
    @MainActor
    final class Silent: TerminalSession {
        let programName = "zsh"
        let isAlternateScreen = false
        var connectionState: TerminalConnectionState = .connecting(mac: "Mac Studio")
        func start(_ handler: @escaping @MainActor (TerminalSessionEvent) -> Void) {}
        func send(_ bytes: Data) {}
        func resize(columns: Int, rows: Int) {}
        func stop() {}
    }

    /// A page with rows on it: light glyph-sized strokes on a dark background.
    static func cachedFrame() -> UIImage {
        UIGraphicsImageRenderer(size: CGSize(width: 390, height: 300)).image { context in
            UIColor(white: 0.08, alpha: 1).setFill()
            context.fill(CGRect(x: 0, y: 0, width: 390, height: 300))
            UIColor(white: 0.9, alpha: 1).setFill()
            for column in 0..<6 { context.fill(CGRect(x: 8 + column * 9, y: 12, width: 2, height: 12)) }
        }
    }

    static func uniform(_ color: UIColor, size: CGSize = CGSize(width: 390, height: 300)) -> UIImage {
        UIGraphicsImageRenderer(size: size).image { context in
            color.setFill()
            context.fill(CGRect(origin: .zero, size: size))
        }
    }

    /// Before-run: the reviewer's route on base, old API only.
    @Test func aPageThatNeverAttachedNeverReplacesTheCachedFrame() {
        let model = TerminalModel(session: Silent(), artifactSource: UnavailableArtifactSource(), title: "interim-coord")
        let cached = Self.cachedFrame()
        model.restoreLastFrame(cached)
        let screen = TerminalViewController(model: model)
        let window = UIWindow(frame: CGRect(x: 0, y: 0, width: 390, height: 844))
        window.rootViewController = screen
        window.makeKeyAndVisible()
        screen.view.layoutIfNeeded()
        defer { window.isHidden = true }
        #expect(screen.snapshot(hidingCursor: true) != nil, "the page in a window has a still")
        // What cool() stores over the cached frame on base.
        let stored = model.frameForSaving() ?? model.snapshot
        #expect(model.frameForSaving() == nil, "a page from before the first attach is never saved")
        #expect(stored === cached, "the tab keeps the cached frame it had")
    }
}
#endif
