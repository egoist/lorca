import XCTest
@testable import Lorca

final class ChannelTests: XCTestCase {
    func testARunnersChannelsAndAConversationDecode() throws {
        let device = try Wire.decoder.decode(Wire.Device.self, from: Data(#"""
        {"id":"runner-1","name":"Workbench","model":"Mac","os":"macos","os_version":"27","machine_key":"mk","is_this_device":true,"status":"online","last_seen":1,
         "channels":[{"id":"ev-1","bot_id":"bot-1","name":"Community feedback","service":"telegram","account_id":"telegram-1",
           "listen":{"mentions":true,"replies":true,"tags":["feedback"]},"task":"File it","state":"held","held_delivery":"d1"}]}
        """#.utf8)).toModel()
        let channel = try XCTUnwrap(device.channels.first)
        XCTAssertEqual(channel.state, .held)
        XCTAssertEqual(channel.heldDelivery, "d1")
        XCTAssertTrue(channel.chats.isEmpty)
        XCTAssertEqual(channel.listen.summary, "Mentions, replies, #feedback")
        XCTAssertEqual(ChannelListen(every: true, tags: ["feedback"]).summary, "Every message")

        let chat = try Wire.decoder.decode(Wire.Chat.self, from: Data(#"""
        {"id":"chat-1","kind":"dm","title":"Acme Community","bot_ids":["bot-1"],"is_pinned":false,"created_at":1,
         "channel":{"channel_id":"ev-1","service":"telegram","account_id":"telegram-1","chat_id":"-1001"},
         "messages":[{"id":"ext-1","chat_id":"chat-1","author":{"kind":"contact","name":"Alice Chen"},"body":{"kind":"text","text":"#feedback it crashes"},"state":{"kind":"complete"},"created_at":1,"external_id":"41"}]}
        """#.utf8)).toModel()
        // A channel's conversation is named after where it happens and is not the bot's DM.
        XCTAssertEqual(chat.customTitle, "Acme Community")
        XCTAssertEqual(chat.channel?.chatID, "-1001")
        XCTAssertTrue(chat.isDM)
        XCTAssertFalse(chat.isBotDM)
        XCTAssertTrue(chat.showsSpeakers)
        XCTAssertEqual(chat.messages.first?.author, .contact("Alice Chen"))
    }
}
