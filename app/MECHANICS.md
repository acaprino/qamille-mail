# Mechanics sheet: Spark Desktop 3.32.2 → Qamille

## Where the logic lives (decompiled)
`Spark Desktop.exe` was opened and searched: it is the Chromium/Node runtime (40,908 functions found;
0 strings for "snooze", no mail logic; evidence `ev_41ae43fa…`). The real logic is in native Swift
libraries under `resources/app.asar.unpacked/node_modules/@vendor/sparkcore-win/bin/Release/SparkCore.bundle/`.
All addresses below are in those libraries. Unless another library is named, the address is in
`SmartMailCore.dll`. Decompiled with the built-in decompiler, because Ghidra failed on this program;
its output is register-level, so rules are my reading of it. Interface words come from `resources/app.asar`,
cited by bundle file. Nothing was copied as code: every rule was rewritten in Rust from this sheet.

## Smart Inbox cards
| Rule | Value | Source |
|---|---|---|
| Default card order | Priority, New, Notifications, Newsletters, Pins, Seen | `RSMListConfigurationFactory.defaultSmartInbox` 0x180b2b110 → sub_180b2b4c0 builds raw types 15, 7, 1, 2, 3, 11, 4, 5 |
| Left out here | raw 7 sharedDraftNotifications, raw 11 channels | team features, not in this version |
| Raw type names | new 1, notifications 2, newsletters 3, pins 4, seen 5, ... channels 11, priority 15 | reflected case list in `.sw5rfst`, aligned so that unlimited Seen comes last (inferred) |
| Items per card | New 5, others 3, Seen unlimited | sub_180b2b3c0 (default 3, raw 5 unlimited); sub_180b2b4c0 sets raw 1 to 5 |
| Show more step | 10 | `maxVisibleItems` init 0x180afb650. How the window uses it was not traced: **estimated** use |
| Card for a mail | unread goes by category, read goes to Seen | index `conversation_index_smart_inbox_new` (type, inInbox, unseen, category) 0x181cdf630; **inferred**, the card query itself was not read |
| Opened mail | keeps its card while it's the open one; marking it read moves it only when you open another. Pinning moves it at once | `LockingMessagesDataSource.lockedMessageQueryFlagsToIgnore` 0x180687f10 and locked flags sub_180532090 (seen, starred, delegated). When the lock ends is **estimated** |
| Sort order | `inboxOrSnoozeDate` newest first, so returned snoozes sort as of their return | index `conversation_index_inbox` 0x181cdf410 |
| Unified view title | "All Accounts" | renderer `smartInboxAll` |
| Categories | personal, priority, notification, newsletter, invitation, invitation_response | `SKILL.md` filter `category:` |

## Machine-mail checks
| Check | Rule | Source |
|---|---|---|
| isNoReplySender | address is no-reply@, noreply@ or donotreply@ | rule text 0x181f3fa30 |
| isBulkMail | `Precedence: bulk/list` or `List-Unsubscribe` | same text; headers listed in sub_180e451c0 |
| isAutoSubmitted | `Auto-Submitted` header present and not `no` | same text; sub_180e451c0. "not no" is the RFC 3834 meaning |
| isCalendarMessage | .ics attachment or text/calendar part | same text |
| Other offline word rules | subject and body keywords | **estimated**: the original's classification also comes from its server |

## Email actions ("emails as tasks")
From `SKILL.md` § action: pin/unpin, mute, snooze/unsnooze, changeReminder/clearReminder
(follow-up if no reply), setAside, archive, moveToInbox, moveToTrash, moveToFolder,
attachLabel/detachLabel, markAsDone/markAsUndone, markAsSeen/markAsUnseen, markAsSpam,
markThreadAsPriority, unsubscribe, assign, delegationComplete/Reopen, send (with optional
Send Later date).

Task view (Qamille): an email becomes a task when it is pinned, has a due date
(`due`/`overdue` strings in `15834_*.js`), a reminder or is set aside. "Done" removes it from the
inbox, like `markAsDone`.

## Snooze / Reminder / Send Later
Kind numbers come from the reflected case list at 0x18295f7d0: today 0, tomorrow 1, tomorrowEve 2, nextWeek 3,
weekend 4, inAMonth 5, someday 6, pickDate 7, tonight 8, lastPicked 9, laterThisWeek 10.
| Rule | Value | Source |
|---|---|---|
| Later Today | now + 10800 s (3 h), capped at midnight, rounded to the nearest 1800 s; if not in the future, + 1800 s | `snoozeTimeModifier` 0x18118a050 (0x2a30); `date(kind:baseDate:)` 0x18118ddd0; `evening` = 23:59:59 in RDDateLibSwift.dll 0x180002d10 |
| This Evening | 20:00 today (72000 s); not offered once past | 0x18118a050 (0x11940); 0x18118ddd0 returns nothing when past |
| Tomorrow | 09:00 next day (32400 s) | 0x18118a050 (0x7e90) |
| Tomorrow Evening | 20:00 next day | 0x18118a050 (0x11940) |
| Weekend | 09:00 on the last day of the week; if that is today, a week later | default weekday set at 0x18118eb20; `dateByApplyingTimeModifier` 0x18118e030 |
| Next Week | 09:00 on the first day of the week, in a later week | 0x18118eb20; 0x18118e030 |
| First day of the week | Monday. The original asks the system | **estimated** for this PC |
| In a Month | one calendar month ahead, 09:00 | 0x18118ddd0 (`byAddingMonths(1)`); 0x18118a050 |
| Later This Week | 86400 s = next day at 00:00, capped at the end of the week | 0x18118a050 (0x15180); 0x18118e030. Off by default in the original |
| Someday | now + 3153600000 s (100 years) | `somedaySnoozeDate` 0x180da7f90, constant at 0x181d7a648 |
| Counts as Someday | at least 315360000 s (10 years) ahead | `isSomedaySnooze` 0x180da7fb0, constant at 0x181d7a650 |
| Snooze menu | Later Today, This Evening, Tomorrow, Weekend, pick a date | `f_defaultSnooze` 0x18118e860 (kinds 0, 8, 1, 4, 7) |
| Reminder menu | the same plus Next Week; at most 60 days ahead | `f_defaultReminder` 0x18118e8c0 (adds kind 3; field 0x28 = 0x3c) |
| Send Later menu | same as Snooze; at most 180 days ahead | `f_defaultSendLater` 0x18118e940 (field 0x28 = 0xb4) |
| Other presets | under "More" | can be turned on in the original's settings; order from `chunk__63094` module 784300 |
| Snoozed mail returns | "Snoozed message returned to your Inbox" | renderer `snoozeSingleMessageNotificationText` |

## Search & filters
| Rule | Value | Source |
|---|---|---|
| Operators | from:, to:, cc:, subject:, before:, after: (yyyy/MM/dd), newer_than:/older_than: (d,w,m,y), has:attachment, is:unread/read/starred/pinned/unreplied, category:, filename: | `SKILL.md` § emails |
| Keyword search result cap | 20 | `SKILL.md` § search |
| Default page size | 20 | **estimated** |
| Streamed attachment cap | 25 MB | `SKILL.md` § draft `--attach-stream` |

## +AI
| Rule | Value | Source |
|---|---|---|
| Writing commands | Expand, Shorten, Proofread, Rephrase, Break into paragraphs, Friendly tone, Formal tone, Neutral tone | `15834_*.js` `aiSystemCommandName*` |
| Summary commands | Short summary, Detailed summary, Action Points | same |
| Auto labels | label suggested per mail, "Wrong label?" feedback | `autoLabelFeedbackDialogTitle` |
| Assistant tools | search emails, calendar events, create/edit event, free slots, contacts, read open email, read attachment, read meeting transcript, message action, draft email | `aiAssistantStatus*` strings |
| Provider | Qamille: any OpenAI-compatible endpoint, Anthropic, or Ollama; offline rules when no key | design choice |

## Calendar & meetings
| Rule | Value | Source |
|---|---|---|
| Video conference kinds | meet, zoom, teams, auto | `SKILL.md` § event `--video-conference` |
| RSVP | accept / decline / maybe (iTIP REPLY) | `SKILL.md` § event |
| Meeting note tabs | Summary, Transcript, Notes (+ Info) | `meetingTranscriptTab*` |
| Summary sections | Key Points, Action Items | `meetingTranscriptSummaryKeyPoints/ActionPoints` |
| Recorder controls | Pause, Resume, Finish ("Is your meeting over?") | `meetingTranscriptPopup*` |
| Ad-hoc meeting title | "Ad-Hoc Meeting" | `meetingTranscriptInstantEventTitle` |
| Online meeting link (Qamille) | Jitsi room link generated locally | design choice (no account needed) |

## Agent access levels (MCP server)
| Level | Allows | Source |
|---|---|---|
| read-only | list/search/read mail, events, contacts, meetings | `SKILL.md` § accounts |
| triage | + drafts, actions | same |
| send | + sending, calendar changes | same |
Qamille default: triage. **estimated**

## Accounts: Google only
The person asked to support only Gmail / Google for now, so other providers were removed.
| Rule | Value | Source |
|---|---|---|
| Incoming | imap.gmail.com:993 (TLS) | Google's published settings |
| Outgoing | smtp.gmail.com:465 (TLS) | Google's published settings |
| Sign-in | Google OAuth 2.0 in the browser with PKCE and a local return address. Refresh token in Qamille's credential vault in the data folder (`credentials.json`, encrypted with a key that Windows protects for the user with DPAPI, `credentials.key.dpapi`; an older plain `credentials.key` is wrapped at the first start), never in the mail data file. IMAP and SMTP use XOAUTH2. When Google refuses the refresh token (`invalid_grant`), the error keeps the code `sign_in_expired` (`oauth::provider_error`) and a Windows notification asks to sign in again (docs/aspetto-nativo.md, rule 28) | SmartMailCore.dll: auth address at file offset 29916400 |
| Auth address | https://accounts.google.com/o/oauth2/v2/auth | SmartMailCore.dll, same place |
| Scopes | https://mail.google.com and userinfo.email. The original also asks for calendar, directory.readonly and userinfo.profile, not used here yet | SmartMailCore.dll, file offset 29916032 |
| Token address | https://oauth2.googleapis.com/token | Google's documented address; not found in the original's strings |
| Client | Qamille's Desktop OAuth client, configured by the developer for the build. A personal project can be entered in Settings → Accounts → Developer settings; the in-app setup guide links to each Google Cloud step. The original's client ID is not reused. See `docs/oauth-google.md` | design choice |
| Other way in | Google app password, kept as an option | design choice |
| Before adding an account | real sign-in for receiving and for sending; nothing changes unless Google accepts both | design choice |
| Archive / Trash folders | asked from Gmail by role (`\All`, `\Trash`), because their names follow the account language | IMAP special-use |
| Labels to Gmail | copy to the label's folder; off by default | Gmail IMAP behaviour |
| Auto sync | every 300 s | **estimated** |
| Calendar | stored on this computer, no Google Calendar sync (it would need OAuth) | limit of this version |
| Google Meet | opens meet.google.com/new; the link is pasted into the event | limit of this version (the Meet API needs OAuth) |

## Semantic search (local RAG)
The original has an on-device embedding model too: SparkAIModels.dll `OnnxEmbeddingModel`, file `onnx_emb_fp16.onnx`
from the vendor's own download server (0x1800286c0), a BERT tokenizer with a 30,522-word vocabulary, and 768 values per text
(0x180003fc0). The person asked for EmbeddingGemma 2 instead, so only its chunking is reused.
| Rule | Value | Source |
|---|---|---|
| Model | EmbeddingGemma 2, text model, `q4f16` build, Apache 2.0 | onnx-community/embeddinggemma-2-ONNX, revision daa72c51 |
| Size | 157 MB weights + 32 MB tokenizer + 0.5 MB graph = 190 MB download | measured; smallest of q4f16, q4 (174 MB), quantized (314 MB) |
| Runtime | ONNX Runtime on CPU (needs AVX2), linked into the app | `ort` 2.0.0-rc.15 |
| Graph inputs | input_ids, attention_mask, and empty image/video/audio_features (0 x 512) | read from the model file |
| Graph output | sentence_embedding, 768 values, truncated to 256 and normalised again | model card. The original keeps 768; 256 keeps the index small, as the person asked for the smallest setup |
| Query prompt | `task: search result \| query: {query}` | model card |
| Document prompt | `title: {title} \| text: {content}` | model card |
| Speed and memory | about 83 ms per short text, about 130 MB of memory | measured on this PC, 4 threads |
| q4f16 quality check | no invalid values; cosine 1.000 against q4 on 8 sample texts | measured. The model card warns against float16; this build passed |
| Passages | 256 tokens, 20 overlap, prefer a sentence end | SparkAIModels.dll: `embeddings(input:chunked:)` sub_180002bc0 calls the splitter with 0x100 and 0x14. Sentence preference **estimated**, standing in for its RecursiveCharacterTextSplitter |
| Result ranking | best passage per mail, within 0.12 of the top score, then rank fusion with exact-word search (constant 60) | **estimated** |
| Assistant context | 8 passages, plus upcoming events and meeting summaries | **estimated** |
| Covered | mail text and readable attachments, not Trash or Spam | design choice |
| Privacy | search runs locally. The assistant sends the matched passages to the AI chosen in Settings, unless that is Ollama or built-in rules | design choice |

## Meeting recording limit
The recorder captures the default microphone only, not the other participants' sound from the computer.

## Look
Colours come from the original's `renderer/app-colors.css`: accent `--accentBlue` #157efb (dark #46a2ff), text `--mainText` #141414 (dark #dee3eb), secondary text `--textSecondary` #66707e, sidebar `--sidebarBackground` #282a2e with `--sidebarText` #aeb8c5 in both themes, hover `--gray0` #f5f6f8, red `--red4` #e43535.
Window layout (sidebar, list, reading pane) is **estimated**: no picture of the original's window was captured, because it would show the person's real mail.

## Signatures
| Rule | Value | Source |
|---|---|---|
| Storage | a list of signatures, each with a name and a body | `RSMSignature` 0x1804bd970, 0x1804bda80 (identifier, htmlContent) |
| Link to a mailbox | one default per mailbox: Most Recent, No Signature or a chosen one | `RSMSignatureManager` bindings 0x180f26be0, names `boundSignatureIdentifier`, `lastUsedSignatureIdentifier`, `NoSignatureIdentifier` 0x1804bd900; renderer `settingsComposerDefaultSignatures`, `settingsComposerSignaturesMostRecent`, `settingsComposerSignaturesNoSignature` |
| Default for a new mailbox | Most Recent. With nothing used yet, the newest signature | **estimated** |
| Changing mailbox in a draft | the draft takes that mailbox's signature | `ChangeSignatureCommandExecutor`, `ComposerMailboxSignatureProvider` in SparkComposer.dll; **inferred** |
| Reply | your other messages in the conversation are searched; the longest known signature found wins; otherwise the mailbox default | `SignatureForReplyQuery.preferred...` 0x1803c8890; helper 0x1803c93b0 (skips the message itself, keeps the longest match by length). Helper 0x1803c8d20 also looks for a marked signature part in a message; here mail is plain text, so only the text match applies. "Your" messages: filter sub_1803c9a60 was not read, so **inferred**. Only messages sent from this app are known locally, because the Sent folder isn't downloaded |
| Message order | message, signature, quote | `DraftFullBodyGeneratorImpl.generate` SparkComposer.dll 0x180065180. Its "Sent with Spark" line is left out |
| Signature attachments and images | not in this version | SparkComposer.dll has `SignatureAttachmentsHelper` |

## Editor
| Rule | Value | Source |
|---|---|---|
| Modes | Plain text, or HTML with a visual editor: no codes while writing. A Source button shows the code | the person's request. The original edits visually too, with CKEditor (renderer CSS `.ck-editor__editable`, `.composer-main-root`); renderer `settingsSignaturesShowHtml` for HTML |
| Toolbar | Bold, Italic, Underline, Strikethrough, Code, Heading, Bullet list, Numbered list, Quote, Indent, Hyperlink, Text color, Highlight color, Clear Formatting | renderer `toolbar*` strings. Code is an addition |
| Shortcuts | Ctrl+B, Ctrl+I, Ctrl+U | **estimated** |
| Lists | Enter continues a list; Enter on an empty item ends it | **estimated**, usual editor behaviour |
| What is sent | plain text alone in Plain text mode; text and HTML together otherwise | standard multipart/alternative |
| Reading HTML | headings, paragraphs, lists, quotes, bold, italic, underline, strike, code, links, colours and indent are kept; scripts and styles are dropped, other tags are ignored | design choice |
| Safety | links only http, https, mailto; colours only as #hex | design choice |
| Bold font | Segoe UI Bold from Windows, because the bundled font has one weight | design choice |

## Decompiling coverage
Besides the libraries above, the main program was also opened: its entry 0x142fa5700 just starts the runtime, and it loads `app.asar` (string at 0x14b3d5f7b). Searching it for snooze, newsletter, reminder and Gmail words finds nothing about mail (`ev_41ae43fa…`, `ev_f23a0c11…`), so no mail rules live there. `RSMSmartInboxCardConfiguration.listContentDescriptor` 0x18050dab0 only covers one card type (raw 0x1a), and sub_18051fbc0, the largest caller of `ListContentFlagBasedCriteria` 0x180532690 that was read, builds the drafts list. The query behind each Inbox section wasn't found, so which mail goes in which section stays inferred.

## Message window and footer tabs
| Rule | Value | Source |
|---|---|---|
| Window modes | small window in the corner, or full screen | renderer module 517302: `Pip`, `Full`, `FullCompact`, `Signature`, `Template`; label `expandComposerWindow` "Full screen" |
| Small window minimum | 550 x 395 | renderer: `--pipMinWidth` 550; module 644363 `nQ` = 395 |
| Inner margin | small window 12, full screen 80, narrow full screen 40 | renderer modules 644363 and 737885 (`Qu`, `l3`, `YX`) |
| Small window size | 640 x 620 when there is room, 12 from the edges | **estimated** |
| Narrow full screen below | 900 wide | **estimated** |
| Tabs in the footer | one per message; click opens or collapses; cross saves and closes | the person, describing the original. No tab code was found in the original's files, so looks and behaviour are **estimated** |
| Open at once | one; opening another collapses the first | **estimated** |
| Autosave of open messages | every 30 s, and on collapse, switching and quitting | **estimated** |
| Closing | saves as draft, unless nothing was written | renderer `composerSaveCloseDraft` "Save and Close Draft", `composerCloseComposer` |
