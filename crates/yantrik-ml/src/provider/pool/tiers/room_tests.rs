use super::*;

#[test]
fn a_free_groq_model_takes_what_its_minute_allows_and_a_known_context_otherwise() {
    let groq = Room::of_tier(tier("groq"), "openai/gpt-oss-120b");
    assert_eq!(groq, Room { per_minute: Some(8_000), context: Some(131_072), requests_per_minute: Some(30) });
    assert_eq!(groq.per_request(), Some(8_000), "the smaller of the two");
    assert_eq!(groq.prompt_limit(2_000), Some(8_000), "the minute counts the prompt, not the answer");
    assert_eq!(Room::of_tier(None, "anything"), Room::default(), "an account that states nothing is not held to anything");
    assert_eq!(Room::default().per_request(), None);
}

#[test]
fn an_answer_gets_a_quarter_of_the_room_and_never_more_than_the_ceiling() {
    assert_eq!(Room { per_minute: Some(8_000), ..Room::default() }.answer_room(32_768), 2_000);
    assert_eq!(Room { context: Some(200_000), ..Room::default() }.answer_room(32_768), 32_768);
    assert_eq!(Room::default().answer_room(32_768), 32_768, "unknown room: the ceiling");
}
