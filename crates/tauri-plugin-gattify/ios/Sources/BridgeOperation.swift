import Foundation

typealias ReplyHandler = (Result<Reply, BridgeError>) -> Void

/// One `execute` call. It answers exactly once: with a reply, a rejection, or an abort.
final class BridgeOperation {
  let id: String
  let owner: String
  private(set) var deadline: DispatchTime?
  private var reply: ReplyHandler?
  private var timer: DispatchWorkItem?
  /// Stops the procedure when a deadline, `cancel` or `closeOwner` ends the operation early.
  var onAbort: ((BridgeError) -> Void)?
  var onFinish: ((BridgeOperation) -> Void)?

  init(id: String, owner: String, reply: @escaping ReplyHandler) {
    self.id = id
    self.owner = owner
    self.reply = reply
  }

  var isFinished: Bool { reply == nil }

  func arm(milliseconds: UInt64, on queue: DispatchQueue) {
    let deadline = DispatchTime.now() + dispatchInterval(milliseconds: milliseconds)
    let timer = DispatchWorkItem { [weak self] in self?.abort(.timeout) }
    self.deadline = deadline
    self.timer = timer
    queue.asyncAfter(deadline: deadline, execute: timer)
  }

  func resolve(_ value: Reply) {
    answer(.success(value))
  }

  func reject(_ error: BridgeError) {
    answer(.failure(error))
  }

  /// Rejects the operation, then runs its cleanup.
  func abort(_ error: BridgeError) {
    guard !isFinished else { return }
    let cleanup = onAbort
    answer(.failure(error))
    cleanup?(error)
  }

  func answer(_ result: Result<Reply, BridgeError>) {
    guard let reply else { return }
    self.reply = nil
    timer?.cancel()
    timer = nil
    onAbort = nil
    let finished = onFinish
    onFinish = nil
    finished?(self)
    reply(result)
  }
}
