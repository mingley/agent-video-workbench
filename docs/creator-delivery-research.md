# Research: creator discovery and delivery

Reviewed November 2, 2026. These observations come from publicly available
documentation, not comparative product tests or interviews with creators.
They identify useful workflow requirements; the tool does not upload to a
platform or claim that any platform will accept an output.

| Evidence | Implication for an agent workflow |
| --- | --- |
| [Adobe's text-based editing documentation](https://helpx.adobe.com/premiere/desktop/edit-projects/edit-video-using-text-based-editing/overview-of-text-based-editing.html) describes searching source transcripts to navigate and select clips. [Unicode UAX #15](https://www.unicode.org/reports/tr15/) explains canonical equivalence of different character sequences. | Phrase search should offer an explicit Unicode-normalized mode and a way to join cue boundaries in scripts where a space is inappropriate. Both should remain literal text searches with source cue identities and timing, not claims of speech understanding or word alignment. |
| [YouTube's caption guidance](https://support.google.com/youtube/answer/2734796?hl=en) describes timestamped files and the need to include relevant sounds. [W3C's caption guidance](https://www.w3.org/WAI/media/av/captions/) explains that captions also represent meaningful non-speech information. | Editable sidecars help creators review and correct captions, but a file's presence cannot establish accuracy, completeness or accessibility. Invalid or unrepresentable cue times should be reported rather than silently exported. |
| [YouTube's chapter guidance](https://support.google.com/youtube/answer/9884579?hl=en) describes manually authored timestamps and titles, including a zero starting point, at least three ascending timestamps, and a minimum chapter length; eligibility may depend on the channel and video. | Let creators select and name chapters from a frozen render, show the exact source marker time and rounding, and report the published structural checks separately from account eligibility. Never expose unselected private markers or invent titles. |
| [TikTok's creator-info API](https://developers.tiktok.com/docs/en/content-posting-api-reference-query-creator-info) reports a maximum video duration that can differ between creators. [YouTube's upload guidance](https://support.google.com/youtube/answer/1722171?hl=en) gives technical recommendations in a particular product context. | Accept the creator's own constraints and check measured render properties and file bytes. Do not hard-code a universal platform duration or treat technical checks as upload approval. |

The implemented contracts and exact limitations are in the
[transcript-search guide](transcript-search.md), the [API reference](agent-api.md),
and [implementation status](implementation-status.md). Audio/video quality,
editorial intent, account access, actual upload, real-device appearance and
caption correctness require separate review. The
[creator opportunities](creator-agent-opportunities.md) plan lists further
work, including creator trials with permitted media.
