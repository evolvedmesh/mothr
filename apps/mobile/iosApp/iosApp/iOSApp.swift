import SwiftUI
import ComposeApp

/// Thin native host: all UI lives in Kotlin (composeApp/src/commonMain), all logic in Rust
/// (crates/mothr-client). This file puts the Compose view controller on screen and wires up
/// notifications and background refresh (Background.swift).
@main
struct MothrApp: App {
    @UIApplicationDelegateAdaptor(AppDelegate.self) private var delegate
    @Environment(\.scenePhase) private var phase

    init() {
        // Before anything touches the shared model.
        Mothr.shared.background = NotificationBridge.shared
        // Create the model (and reconnect a paired phone) outside any Compose snapshot.
        _ = Mothr.shared.model
    }

    var body: some Scene {
        WindowGroup {
            ComposeView().ignoresSafeArea(.all)
        }
        .onChange(of: phase) { phase in
            switch phase {
            case .active: NotificationBridge.shared.enterForeground()
            case .background: NotificationBridge.shared.enterBackground()
            default: break
            }
        }
    }
}

final class AppDelegate: NSObject, UIApplicationDelegate {
    func application(_ application: UIApplication, didFinishLaunchingWithOptions launchOptions: [UIApplication.LaunchOptionsKey: Any]? = nil) -> Bool {
        // Background tasks must be registered before launch finishes.
        NotificationBridge.shared.register()
        return true
    }
}

struct ComposeView: UIViewControllerRepresentable {
    func makeUIViewController(context: Context) -> UIViewController {
        MainViewControllerKt.MainViewController()
    }

    func updateUIViewController(_ uiViewController: UIViewController, context: Context) {}
}
