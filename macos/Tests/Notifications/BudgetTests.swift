import XCTest
@testable import Lorca

@MainActor
final class BudgetTests: XCTestCase {
    private func usage(_ kinds: [String], api: Double = 0, estimate: Double = 0, unknown: Int = 0) -> ChatUsage {
        ChatUsage(contextTokens: 0, contextWindow: 0, inputTokens: 0, outputTokens: 0, cacheReadTokens: 0, costUSD: 0, turns: 3, model: "test",
                  apiCostUSD: api, subscriptionEstimateUSD: estimate, unknownPriceCalls: unknown, pricingKinds: kinds)
    }

    func testSpentKeepsAPISpendingEstimatesAndUnknownPricesApart() {
        XCTAssertEqual(usage(["api"], api: 0.42).spendSummary, "$0.42 · 3 turns")
        XCTAssertEqual(usage(["subscription_estimate"], estimate: 0).spendSummary, "$0.00 est. · 3 turns")
        XCTAssertEqual(usage(["api", "subscription_estimate"], api: 0.03, estimate: 0.4).spendSummary, "$0.03 + $0.40 est. · 3 turns")
        XCTAssertEqual(usage(["unknown"], unknown: 2).spendSummary, "Price unknown · 3 turns")
        XCTAssertEqual(usage([]).spendSummary, "Price unknown · 3 turns", "no category is never $0.00")
        XCTAssertEqual(usage(["api", "unknown"], api: 0.1, unknown: 1).spendSummary, "$0.10 + unknown · 3 turns")
    }

    func testAStoppedTurnSaysWhichLimitAndResumesOnlyWhenThatLimitWentUp() throws {
        let data = Data(#"{"kind":"job","id":"job-1","runner_id":"runner","bot_id":"bot","chat_id":"chat","job_kind":"turn","limits":{"max_tokens":100,"max_runtime_secs":600},"usage":{"tokens":100,"api_cost_usd":0.02,"subscription_estimate_usd":0,"unknown_price_calls":0,"estimated_calls":0,"model_calls":4,"runtime_secs":20,"retries":1,"connector_calls":2},"state":"budget_exhausted","reached":"tokens","reason":"Stopped at the token limit.","updated_at":1}"#.utf8)
        let state = try Wire.decoder.decode(BudgetState.self, from: data)
        XCTAssertTrue(state.isStopped)
        XCTAssertEqual(state.stoppedTitle, "Token limit reached")
        XCTAssertEqual(state.stoppedDetail, "Used 100 of 100 tokens.")
        XCTAssertFalse(state.fits(state.limits), "the same limits need a fresh allowance")
        XCTAssertFalse(state.fits(BudgetLimits(maxTokens: 100, maxRuntimeSecs: 1200)), "raising another limit doesn't help")
        XCTAssertTrue(state.fits(BudgetLimits(maxTokens: 200, maxRuntimeSecs: 600)))
        XCTAssertTrue(state.fits(BudgetLimits(maxRuntimeSecs: 600)), "no token limit at all")
        XCTAssertFalse(state.fits(BudgetLimits(maxTokens: 200, maxRuntimeSecs: 10)), "another limit already used up")
    }

    func testLimitsSummaryAndCallWindows() {
        XCTAssertEqual(BudgetLimits().summary, "None")
        XCTAssertEqual(BudgetLimits(maxUsd: 2, maxTokens: 200_000).summary, "$2.00 · 200k tokens")
        XCTAssertEqual(BudgetLimits(maxRuntimeSecs: 900).summary, "15 min")
        XCTAssertEqual(CallLimits(maxCalls: 60, windowSecs: 60, maxConcurrency: 4, sharesService: false).summary, "60 a minute")
        XCTAssertEqual(CallLimits(maxCalls: 10, windowSecs: 30, maxConcurrency: 4, sharesService: false).summary, "10 every 30 s")
    }
}
