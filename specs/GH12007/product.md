# Double-click selection CJK and full-width punctuation boundaries — Product Spec
GitHub issue: https://github.com/warpdotdev/warp/issues/12007
Figma: none provided. This is a terminal text-selection behavior fix with no new visual design.
## Summary
Double-click word selection in terminal text should treat CJK and full-width punctuation as word boundaries, the same way it already treats common ASCII punctuation. A user double-clicking a word adjacent to punctuation such as `，`, `。`, `：`, `「」`, or `（）` should select the word only, not the punctuation or the next neighboring word.
## Problem
Warp currently over-selects when terminal words touch CJK or full-width punctuation. For example, double-clicking `test` in `test，next` selects `test，next`, while double-clicking `test` in `test,next` selects only `test`. This makes terminal output containing East Asian punctuation feel inconsistent with ASCII punctuation and causes copied selections to include unintended text.
The related path-detection bug in #10245 was fixed on a separate code path. This issue covers double-click word selection and must not rely on clickable-link detection behavior to mask the problem.
## Goals
1. Make double-click word selection stop at CJK and full-width punctuation on both sides of a word.
2. Cover the full class of CJK / full-width punctuation, not only the full-width comma.
3. Preserve existing ASCII punctuation and whitespace word-boundary behavior.
4. Preserve CJK letters and ideographs as selectable word content; text such as `你好世界` should not split into single-character selections just because it is CJK text.
5. Preserve existing smart-selection behavior for URLs, file paths, emails, identifiers, brackets, line selection, rectangle selection, and custom user word-boundary settings.
## Non-goals
1. Do not change file-link detection except where implementation reuse is purely internal and behavior-preserving for links.
2. Do not add a new user setting for Unicode punctuation boundaries.
3. Do not change click counts: single-click, drag selection, triple-click line selection, and rectangle selection keep their current semantics.
4. Do not redefine CJK word segmentation. This fix is about punctuation boundaries, not language-aware segmentation for text with no punctuation or whitespace.
5. Do not change terminal rendering, character width handling, font fallback, or the clipboard format beyond selecting the correct text range.
## Behavior
1. When the user double-clicks within an ASCII or Unicode word that is immediately followed by CJK / full-width punctuation, the selection ends before that punctuation.
   - `test，next` → double-clicking any character in `test` selects `test`.
   - `test。next` → selects `test`.
   - `test：next` → selects `test`.
   - `test）next` → selects `test`.
2. When the user double-clicks within an ASCII or Unicode word that is immediately preceded by CJK / full-width punctuation, the selection starts after that punctuation.
   - `prev，test` → double-clicking any character in `test` selects `test`.
   - `prev「test` → selects `test`.
   - `prev（test` → selects `test`.
3. When CJK / full-width punctuation appears between two words, double-clicking either word selects only that word and does not cross the punctuation.
   - `alpha，beta` → double-clicking `alpha` selects `alpha`; double-clicking `beta` selects `beta`.
   - `名前：value` → double-clicking `名前` selects `名前`; double-clicking `value` selects `value`.
4. The fix covers common CJK and full-width punctuation, including at minimum: `，` `、` `。` `．` `！` `？` `；` `：` `「` `」` `『` `』` `（` `）` `《` `》` `〈` `〉` `【` `】` `〔` `〕`.
5. The boundary behavior is category-based rather than comma-specific: non-ASCII Unicode punctuation characters are word boundaries unless a user-facing custom word-character setting explicitly treats that character as part of a word.
6. CJK ideographs, kana, hangul, full-width letters, full-width digits, accented Latin letters, Cyrillic, Arabic, and other non-punctuation letters or numbers remain part of words. Double-clicking `你好世界` selects `你好世界`, not a single ideograph. Double-clicking `ｔｅｓｔ１２３` selects `ｔｅｓｔ１２３`, not individual full-width characters.
7. Whitespace remains a word boundary exactly as before, including Unicode whitespace that Warp already treats as whitespace.
8. Existing ASCII punctuation behavior is unchanged. Inputs such as `test,next`, `test.next`, `test/next`, `test-next`, and `test:next` keep selecting the same ranges they select today.
9. Smart Select remains enabled by default and may still expand a double-click to a larger recognized object, such as a URL, email address, file path, or identifier, when that object contains the click position and the smart match is larger than the fallback word selection.
10. Smart Select must not include adjacent CJK / full-width punctuation unless the recognized object itself legitimately includes that character. For example, in `「https://warp.dev」`, double-clicking inside the URL selects `https://warp.dev`, not the surrounding brackets.
11. When Smart Select is disabled, fallback word selection still treats CJK / full-width punctuation as boundaries by default.
12. If a user has configured a custom word-character allowlist and explicitly includes a punctuation character that would otherwise be a boundary, that custom setting continues to make the character part of the selected word in the surfaces where the setting applies.
13. The behavior applies to terminal command output and prompt/command text selection. The same boundary helper may also affect shared selectable text surfaces that intentionally use terminal/editor word-boundary settings, but the required user-visible fix is terminal double-click selection.
14. Wrapped terminal lines do not change the boundary semantics. If a word adjacent to CJK / full-width punctuation wraps across display rows, double-click selection still stops at the punctuation and still handles wide-character spacer cells correctly.
15. Copying the selection returns exactly the selected text. For the reproduction command `printf 'test，next\ntest,next\n'`, double-clicking `test` on either line and copying should put `test` on the clipboard.
16. Bracket-pair semantic selection remains intact for ASCII bracket pairs. This fix does not require adding bracket-pair matching for CJK brackets; it only requires CJK brackets to act as word boundaries around words.
17. The fix is platform-independent. The issue was reported on macOS, but terminal double-click word-boundary behavior should be consistent across supported platforms.
## Success criteria
1. The reporter's reproduction selects `test` for both `test，next` and `test,next`.
2. The examples in Behavior 4 are covered by automated regression tests or an equivalent category-based test that proves they are included.
3. CJK text without punctuation, such as `你好世界`, remains selectable as one continuous word under the default behavior.
4. Existing tests for ASCII word boundaries, smart selection, custom word-character allowlists, URLs, file paths, and wide-character selection continue to pass.
5. The implementation does not add a new visible setting, prompt, or UI state.
## Validation
1. Add automated tests for default word-boundary detection with representative CJK / full-width punctuation on both the left and right side of a clicked word.
2. Add automated tests that demonstrate non-punctuation CJK/full-width letters and digits remain part of words.
3. Add terminal semantic-selection regression tests using the issue reproduction text `test，next` and `test,next`.
4. Add at least one smart-selection regression test showing surrounding CJK punctuation does not get included around a URL or path.
5. Manually verify in Warp by running `printf 'test，next\ntest,next\n'`, double-clicking `test` on both lines, and confirming the selected/copied text is `test`.
## Open product questions
No open product questions. The expected behavior is to align CJK / full-width punctuation with existing ASCII punctuation boundaries while preserving CJK letters and ideographs as word content.
