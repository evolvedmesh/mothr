import BackgroundTasks
import ComposeApp
import UIKit
import UserNotifications

/// iOS side of the Kotlin `Background` hook (composeApp/src/commonMain/.../Mothr.kt).
///
/// iOS doesn't let an app hold a socket open in the background, so Mothr:
/// - keeps the connection for a short grace period after you leave the app,
/// - checks in through background app refresh (iOS decides how often),
/// - posts a local notification for every permission request or question it finds, with the
///   quick replies as notification actions.
final class NotificationBridge: NSObject, Background, UNUserNotificationCenterDelegate {
    static let shared = NotificationBridge()
    static let refreshTask = "dev.mothr.mobile.refresh"

    private let center = UNUserNotificationCenter.current()
    /// One category per notification that has quick replies (iOS actions are per category).
    private var categories: [String: UNNotificationCategory] = [:]
    private var grace: UIBackgroundTaskIdentifier = .invalid

    private var model: AppModel { Mothr.shared.model }

    // MARK: Background

    func keepAlive(on: Bool) {
        if on {
            center.requestAuthorization(options: [.alert, .sound, .badge]) { _, _ in }
            scheduleRefresh()
        } else {
            BGTaskScheduler.shared.cancel(taskRequestWithIdentifier: Self.refreshTask)
        }
    }

    func status(text: String) {}

    func show(item: Attention, source: NotificationSource) {
        let content = UNMutableNotificationContent()
        content.title = item.title
        content.subtitle = [source.label ?? "", item.projectName, item.chatTitle].filter { !$0.isEmpty }.joined(separator: " · ")
        content.body = item.body
        content.sound = .default
        content.threadIdentifier = item.chatId
        content.userInfo = [
            "computer": source.computerId,
            "open_path": item.openPath,
            "chat": item.chatId,
            "entry": item.entryId,
            "kind": item.kind == AttentionKind.permission ? "permission" : "question",
            "replies": item.quickReplies.map { $0.id },
        ]
        if !item.quickReplies.isEmpty {
            let id = "mothr.\(item.key)"
            let actions = item.quickReplies.enumerated().map { i, r in
                UNNotificationAction(identifier: "reply.\(i)", title: r.label, options: r.allow || item.kind == AttentionKind.question ? [] : [.destructive])
            }
            categories[id] = UNNotificationCategory(identifier: id, actions: actions, intentIdentifiers: [])
            center.setNotificationCategories(Set(categories.values))
            content.categoryIdentifier = id
        }
        center.add(UNNotificationRequest(identifier: item.key, content: content, trigger: nil))
    }

    func withdraw(key: String) {
        center.removeDeliveredNotifications(withIdentifiers: [key])
        center.removePendingNotificationRequests(withIdentifiers: [key])
        if categories.removeValue(forKey: "mothr.\(key)") != nil {
            center.setNotificationCategories(Set(categories.values))
        }
    }

    func failed(text: String) {
        let content = UNMutableNotificationContent()
        content.title = "Couldn't send your answer"
        content.body = text
        center.add(UNNotificationRequest(identifier: "failed", content: content, trigger: nil))
    }

    // MARK: Lifecycle

    func register() {
        center.delegate = self
        BGTaskScheduler.shared.register(forTaskWithIdentifier: Self.refreshTask, using: nil) { task in
            self.handleRefresh(task as! BGAppRefreshTask)
        }
    }

    func enterForeground() {
        endGrace()
        model.setForeground(on: true)
    }

    func enterBackground() {
        model.setForeground(on: false)
        scheduleRefresh()
        // Stay connected a little longer, so a request that arrives right after you switch
        // apps still notifies. iOS ends this after ~30 s at most.
        endGrace()
        grace = UIApplication.shared.beginBackgroundTask(withName: "mothr.grace") { [weak self] in self?.endGrace() }
    }

    private func endGrace() {
        if grace != .invalid {
            UIApplication.shared.endBackgroundTask(grace)
            grace = .invalid
        }
    }

    private func scheduleRefresh() {
        guard model.client != nil, model.runInBackground else { return }
        let request = BGAppRefreshTaskRequest(identifier: Self.refreshTask)
        request.earliestBeginDate = Date(timeIntervalSinceNow: 15 * 60)
        try? BGTaskScheduler.shared.submit(request)
    }

    private func handleRefresh(_ task: BGAppRefreshTask) {
        scheduleRefresh()
        task.expirationHandler = { task.setTaskCompleted(success: false) }
        model.checkOnce(timeoutMs: 20_000) { error in
            task.setTaskCompleted(success: error == nil)
        }
    }

    // MARK: UNUserNotificationCenterDelegate

    func userNotificationCenter(_ center: UNUserNotificationCenter, willPresent notification: UNNotification, withCompletionHandler completionHandler: @escaping (UNNotificationPresentationOptions) -> Void) {
        completionHandler([.banner, .list, .sound])
    }

    func userNotificationCenter(_ center: UNUserNotificationCenter, didReceive response: UNNotificationResponse, withCompletionHandler completionHandler: @escaping () -> Void) {
        let info = response.notification.request.content.userInfo
        let action = response.actionIdentifier
        guard let computer = info["computer"] as? String else {
            completionHandler()
            return
        }
        if action.hasPrefix("reply."),
           let index = Int(action.dropFirst("reply.".count)),
           let replies = info["replies"] as? [String], index < replies.count,
           let chat = info["chat"] as? String, let entry = info["entry"] as? String {
            let kind = (info["kind"] as? String) == "permission" ? AttentionKind.permission : AttentionKind.question
            model.quickReply(computerId: computer, chatId: chat, entryId: entry, kind: kind, replyId: replies[index]) { _, _ in
                completionHandler()
            }
            return
        }
        if action == UNNotificationDefaultActionIdentifier, let path = info["open_path"] as? [String] {
            model.openPath(computerId: computer, path: path)
        }
        completionHandler()
    }
}
