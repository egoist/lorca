import XCTest
@testable import Lorca

final class CustomProviderTests: XCTestCase {
    func testTheNoteNamesTheURLTheCLICalls() {
        XCTAssertEqual(CustomAPI.chatCompletions.endpoint(for: " https://openrouter.ai/api/v1/ "), "https://openrouter.ai/api/v1/chat/completions")
        XCTAssertEqual(CustomAPI.chatCompletions.endpoint(for: "http://localhost:11434/v1/chat/completions"), "http://localhost:11434/v1/chat/completions")
        XCTAssertEqual(CustomAPI.responses.endpoint(for: "https://gateway.example/v1/responses/"), "https://gateway.example/v1/responses")
        // Messages adds the /v1 a root without one lacks.
        XCTAssertEqual(CustomAPI.messages.endpoint(for: "https://api.anthropic.com/v1"), "https://api.anthropic.com/v1/messages")
        XCTAssertEqual(CustomAPI.messages.endpoint(for: "https://api.anthropic.com/v1/messages"), "https://api.anthropic.com/v1/messages")
        XCTAssertEqual(CustomAPI.messages.endpoint(for: "https://api.moonshot.ai/anthropic"), "https://api.moonshot.ai/anthropic/v1/messages")
    }

    func testADecisionAPIKeepsTheEndpointItWasGiven() {
        XCTAssertEqual(CustomAPI.systemOne.endpoint(for: "https://api.typesafe.ai/v1"), "https://api.typesafe.ai/v1/systemone")
        XCTAssertEqual(CustomAPI.systemOne.endpoint(for: "https://openrouter.ai/api/alpha/decisions/"), "https://openrouter.ai/api/alpha/decisions")
        XCTAssertEqual(CustomAPI.decisions.endpoint(for: "https://api.openai.com/v1"), "https://api.openai.com/v1/decisions")
        XCTAssertEqual(CustomAPI.decisions.endpoint(for: "https://ai-gateway.vercel.sh/v1/decisions"), "https://ai-gateway.vercel.sh/v1/decisions")
        XCTAssertTrue(CustomAPI.systemOne.decides && CustomAPI.decisions.decides)
        XCTAssertFalse(CustomAPI.allCases.filter { !$0.decides }.contains { $0.decides })
    }

    func testAutoReviewDecodesTheModelItReviewsWith() throws {
        let json = """
            {"is_enabled": true, "rules": [], "provider": "custom:openrouter-decisions",
             "models": {"custom:openrouter-decisions": "typesafe/jev-1.13", "anthropic": "claude-opus-5", "nope": "x"}}
            """
        let review = try Wire.decoder.decode(Wire.AutoReview.self, from: Data(json.utf8)).toModel()
        XCTAssertEqual(review.provider, .custom("custom:openrouter-decisions"))
        XCTAssertEqual(review.models, [.custom("custom:openrouter-decisions"): "typesafe/jev-1.13", .anthropic: "claude-opus-5"])
        let bare = try Wire.decoder.decode(Wire.AutoReview.self, from: Data(#"{"is_enabled": false}"#.utf8)).toModel()
        XCTAssertNil(bare.provider)
        XCTAssertEqual(bare.models, [:])
        let status = try Wire.decoder.decode(Wire.Provider.self, from: Data(#"{"kind": "deepseek", "is_connected": true, "detail": "", "review_model": "deepseek-flash"}"#.utf8))
        XCTAssertEqual(status.toModel()?.reviewModel, "deepseek-flash")
        let jev = try Wire.decoder.decode(Wire.Model.self, from: Data(#"{"provider": "opencode", "id": "jev-1.13", "name": "Jev 1.13", "levels": [], "decides": true}"#.utf8))
        XCTAssertEqual(jev.toModel()?.decides, true)
    }

    func testACustomKindTravelsAsItsWireValue() {
        let kind = ProviderCredential.Kind(wireValue: "custom:openrouter")
        XCTAssertEqual(kind, .custom("custom:openrouter"))
        XCTAssertEqual(kind?.wireValue, "custom:openrouter")
        XCTAssertEqual(kind?.isCustom, true)
        XCTAssertNil(ProviderCredential.Kind(wireValue: "openrouter"))
        XCTAssertFalse(ProviderCredential.Kind.builtIn.contains { $0.isCustom })
    }

    func testACustomModelOffersTheLevelsTheCLIGivesIt() {
        let models = [
            ProviderModel(provider: .custom("custom:lab"), id: "qwen3:8b", label: "qwen3:8b", levels: ["low", "medium", "high"]),
            ProviderModel(provider: .custom("custom:lab"), id: "claude-opus-5", label: "Claude Opus 5", levels: ["off", "low", "medium", "high", "xhigh", "max"]),
        ]
        XCTAssertEqual(ProviderModel.thinkingLevels(for: nil, among: models).map(\.id), ["low", "medium", "high"])
        XCTAssertEqual(ProviderModel.thinkingLevels(for: "claude-opus-5", among: models).map(\.id), ["off", "low", "medium", "high", "xhigh", "max"])
    }

    func testANewListingKeepsPickedAndTypedModelsAndReplacesTheRest() {
        let old = [CustomModel(id: "typed"), CustomModel(id: "picked"), CustomModel(id: "dropped")]
        let listed = [CustomModel(id: "picked", name: "Picked", contextWindow: 128_000), CustomModel(id: "fresh")]
        let merged = ModelChecklist.merge(old, keeping: { ["typed", "picked"].contains($0) }, with: listed)
        XCTAssertEqual(merged.map(\.id), ["typed", "picked", "fresh"])
        XCTAssertEqual(merged[1].contextWindow, 128_000, "the listing's facts replace the kept row's")
        // A server with no list keeps only what was picked or typed, none of the last server's.
        XCTAssertEqual(ModelChecklist.merge(merged, keeping: { $0 == "typed" }, with: []).map(\.id), ["typed"])
    }

    func testTheSearchFieldFiltersOrOffersToAdd() {
        let models = [CustomModel(id: "anthropic/claude-sonnet-5", name: "Anthropic: Claude Sonnet 5"), CustomModel(id: "qwen3:8b")]
        XCTAssertEqual(ModelChecklist.filter(models, "sonnet").map(\.id), ["anthropic/claude-sonnet-5"])
        XCTAssertEqual(ModelChecklist.filter(models, "QWEN").map(\.id), ["qwen3:8b"])
        XCTAssertEqual(ModelChecklist.filter(models, "").count, 2)
        XCTAssertEqual(ModelChecklist.addCandidate(" qwen3:32b ", in: models), "qwen3:32b")
        XCTAssertNil(ModelChecklist.addCandidate("qwen3:8b", in: models), "an id already listed is picked, not added")
        XCTAssertNil(ModelChecklist.addCandidate("  ", in: models))
    }

    func testTheDefaultGoesFirstAndTheRestKeepTheirOrder() {
        let models = ["a", "b", "c", "d"].map { CustomModel(id: $0) }
        XCTAssertEqual(ModelChecklist.orderedIDs(models, selected: ["b", "c", "d"], defaultID: "c"), ["c", "b", "d"])
        XCTAssertEqual(ModelChecklist.orderedIDs(models, selected: ["b", "d"], defaultID: "a"), ["b", "d"], "a default not picked is ignored")
        XCTAssertEqual(ModelChecklist.orderedIDs(models, selected: [], defaultID: nil), [])
    }

    func testAPastedURLFindsItsKnownServer() {
        XCTAssertEqual(CustomProviderPreset.matching("http://localhost:11434/v1/chat/completions")?.name, "Ollama")
        XCTAssertEqual(CustomProviderPreset.matching(" https://openrouter.ai/api/v1 ")?.name, "OpenRouter")
        XCTAssertNil(CustomProviderPreset.matching("http://localhost:8080/v1"), "another port is another server")
        XCTAssertNil(CustomProviderPreset.matching("not a url"))
        // One server's decision API is a preset of its own.
        XCTAssertEqual(CustomProviderPreset.matching("https://openrouter.ai/api/alpha/decisions", api: .systemOne)?.name, "OpenRouter Decisions")
        XCTAssertEqual(CustomProviderPreset.matching("https://openrouter.ai/api/v1", api: .chatCompletions)?.name, "OpenRouter")
        XCTAssertEqual(CustomProviderPreset.matching("https://api.openai.com/v1", api: .decisions)?.name, "OpenAI Decisions")
    }

    func testACustomStatusDecodesWithItsModels() throws {
        let json = """
            {"kind": "custom:lab", "is_connected": true, "detail": "http://lab/v1", "base_url": "http://lab/v1",
             "name": "Lab", "api": "messages", "models": [{"id": "glm-6", "name": "GLM 6", "context_window": 128000, "levels": ["low", "medium", "high"]}, {"id": "qwen3:8b"}]}
            """
        let provider = try Wire.decoder.decode(Wire.Provider.self, from: Data(json.utf8)).toModel()
        XCTAssertEqual(provider?.kind, .custom("custom:lab"))
        XCTAssertEqual(provider?.name, "Lab")
        XCTAssertEqual(provider?.api, .messages)
        XCTAssertEqual(provider?.models, [CustomModel(id: "glm-6", name: "GLM 6", contextWindow: 128_000, levels: ["low", "medium", "high"]), CustomModel(id: "qwen3:8b")])
    }
}
