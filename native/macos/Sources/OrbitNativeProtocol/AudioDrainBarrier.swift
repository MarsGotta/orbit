/// Coalesces concurrent stop requests. Every waiter observes the same completed
/// drain, never merely admission of a stop. No audio, clocks or transport here.
public actor AudioDrainBarrier {
    private var inFlight: Task<Bool, Never>?
    private(set) var waiterCount = 0
    public init() {}

    public func complete(_ drain: @escaping @Sendable () async -> Bool) async -> Bool {
        waiterCount += 1
        defer { waiterCount -= 1 }
        if let inFlight { return await inFlight.value }
        let task = Task { await drain() }
        inFlight = task
        let result = await task.value
        inFlight = nil
        return result
    }
}
