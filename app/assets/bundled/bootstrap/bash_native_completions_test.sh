#!/usr/bin/env bash
# Regression test for the cobra description-padding fix in `_warp_native_bash_completions`
# (see the COMP_TYPE comment in bash_body.sh). Exercises synthetic completion functions rather
# than real `gh`/`git`/`make` binaries, so it runs deterministically without external tools --
# but each one reproduces a real, measured completion script's own behavior:
#
#   - `__cobra_style_complete` reproduces cobra's documented COMP_TYPE branch (see
#     https://github.com/spf13/cobra/issues/1508): under COMP_TYPE 9 (plain Tab, what we always
#     use) with more than one match it bakes a padded "name  (description)" string into
#     COMPREPLY. `_warp_native_bash_completions` is expected to split that shape apart into a
#     bare name and a separate description after the call, not by switching COMP_TYPE.
#   - `__ordinary_style_complete` reproduces a bash-completion-style function that ignores
#     COMP_TYPE entirely and always emits bare names -- the shape the fix must not touch.
#   - `__make_style_complete` reproduces bash-completion's real `make` script, the one
#     completion function (out of 841 in a stock install) that actually reads $COMP_TYPE: it
#     returns the *directory prefix plus* the next path component under COMP_TYPE 9, and just
#     the bare next component under any other COMP_TYPE. Switching COMP_TYPE away from 9 (an
#     earlier version of this fix did, to dodge the cobra padding) silently broke this --
#     completing a prefixed target returned a bare component that no longer contains the typed
#     prefix, so the client's own filter discarded it and no menu appeared at all.
#
# Usage: bash bash_native_completions_test.sh

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../../../.." && pwd)"
failures=0

define_bashpreexec_functions() { :; }
install_bashpreexec() { :; }
WARP_IS_SUBSHELL=1
WARP_SESSION_ID=1
WARP_IS_LOCAL_SHELL_SESSION=0
WARP_USING_WINDOWS_CON_PTY=false
WARP_IN_MSYS2=false
source "$REPO_ROOT/app/assets/bundled/bootstrap/bash_body.sh" >/dev/null 2>&1

__cobra_style_complete() {
  if [[ "$COMP_TYPE" == 9 && ${#COMPREPLY[@]} -ge 0 ]]; then
    # Cobra's actual condition is "more than one match"; reproduce that by hard-coding two
    # matches for this fixture, as the real cobra scripts do for a real multi-match prefix. One
    # entry needs more padding than the other, to catch a regex that over-consumes trailing
    # spaces into the name instead of the padding (measured against the real `gh` binary).
    COMPREPLY=("checkout  (Check out a pull request)" "checks    (Show CI status)")
  else
    COMPREPLY=("checkout" "checks")
  fi
}
complete -F __cobra_style_complete cobra-cli

__ordinary_style_complete() {
  # Deliberately ignores COMP_TYPE, matching bash-completion's own scripts.
  COMPREPLY=("checkout" "cherry-pick" "cherry")
}
complete -F __ordinary_style_complete ordinary-cli

__make_style_complete() {
  if (( COMP_TYPE != 9 )); then
    COMPREPLY=("deploy")
  else
    COMPREPLY=("sub/dir/deploy")
  fi
}
complete -F __make_style_complete make-cli

__semicolon_style_complete() {
  # A literal `;` in a candidate (e.g. a real filename) is not itself unusual -- OSC 9280's
  # own params are semicolon-delimited, and this used to truncate everything after the `;`
  # rather than round-trip it (see decode_hex_completions_payload in ansi/mod.rs).
  COMPREPLY=("semi;colon.txt")
}
complete -F __semicolon_style_complete semicolon-cli

__dashn_style_complete() {
  # A candidate that is exactly `-n`, `-e`, or `-E` is not synthetic -- `kubectl -`, `ssh -`,
  # and `npm -` all offer exactly this as a real flag completion. warp_hex_encode_string used
  # to pipe through `echo`, which treats such an argument as its own flag instead of literal
  # text: `echo -n` prints nothing (encoding to an empty payload) and `echo -e`/`echo -E`
  # print only `echo`'s own trailing newline (encoding to just that byte).
  COMPREPLY=("-n")
}
complete -F __dashn_style_complete dashn-cli

__nested_paren_style_complete() {
  # A cobra-padded description that itself contains a parenthesised, multi-space-padded
  # aside. The greedy version of the split regex backtracked to the *last* multi-space run
  # in the entry, folding part of the description into the name (measured: split as name
  # "cmd    (outer", description "inner) tail" -- both wrong). A second, differently-shaped
  # padded entry is included alongside it because cobra only pads a reply once it has more
  # than one match (see the structural guard in bash_body.sh), so a realistic fixture needs
  # at least two entries to exercise that guard the way a real multi-match cobra reply would.
  if [[ "$COMP_TYPE" == 9 ]]; then
    COMPREPLY=("cmd    (outer  (inner) tail)" "cmdx  (simple)")
  else
    COMPREPLY=("cmd" "cmdx")
  fi
}
complete -F __nested_paren_style_complete nestprobe-cli

__paren_filename_style_complete() {
  # A single real candidate that merely looks padded (e.g. a real file named
  # "Backup  (copy)") must not be split: cobra only pads a reply with more than one match
  # (see the COMP_TYPE=9 comment in bash_body.sh), so this shape with exactly one candidate
  # can only be a real name that happens to contain a parenthesised, multi-space-preceded
  # suffix, never cobra's own padding.
  COMPREPLY=("Backup  (copy)")
}
complete -F __paren_filename_style_complete parenfile-cli

assert_reply() {
  local desc="$1"
  shift
  local -a expected=("$@")
  if [[ "${replies[*]}" != "${expected[*]}" ]]; then
    echo "FAIL: $desc"
    echo "  expected: ${expected[*]}"
    echo "  actual:   ${replies[*]}"
    failures=$((failures + 1))
  else
    echo "PASS: $desc"
  fi
}

assert_descriptions() {
  local desc="$1"
  shift
  local -a expected=("$@")
  if [[ "${descriptions[*]}" != "${expected[*]}" ]]; then
    echo "FAIL: $desc"
    echo "  expected: ${expected[*]}"
    echo "  actual:   ${descriptions[*]}"
    failures=$((failures + 1))
  else
    echo "PASS: $desc"
  fi
}

collect_replies() {
  # _warp_native_bash_completions emits one OSC per match with no separator between them
  # ("\e]9280;C;<match>\a\e]9280;C;<match>\a..."), so extract every match/description with
  # `grep -oP` rather than treating the output as newline-delimited. Each payload is
  # hex-encoded on the wire (see decode_hex_completions_payload in ansi/mod.rs), so decode it
  # back the same way the Rust client does before comparing against plain-text expectations.
  local output
  output="$(_warp_native_bash_completions "$1" 2>/dev/null)"
  local -a hex_replies hex_descriptions
  mapfile -t hex_replies < <(command -p grep -oP '(?<=9280;C;)[^\x07]*' <<< "$output")
  mapfile -t hex_descriptions < <(command -p grep -oP '(?<=9280;D\?description;)[^\x07]*' <<< "$output")
  replies=()
  for hex in "${hex_replies[@]}"; do
    replies+=("$(warp_hex_decode_string "$hex")")
  done
  descriptions=()
  for hex in "${hex_descriptions[@]}"; do
    descriptions+=("$(warp_hex_decode_string "$hex")")
  done
}

collect_replies "cobra-cli che"
assert_reply "cobra-style entry is split to a bare name (no baked-in description)" "checkout" "checks"
assert_descriptions "cobra-style entry's description is recovered, not just discarded" \
  "Check out a pull request" "Show CI status"

collect_replies "ordinary-cli ch"
assert_reply "ordinary bash-completion-style entry is unaffected" "checkout" "cherry-pick" "cherry"

collect_replies "make-cli sub/dir/"
assert_reply "make-style entry keeps the full directory-prefixed path under COMP_TYPE 9" "sub/dir/deploy"

collect_replies "semicolon-cli s"
assert_reply "a literal semicolon in the match text round-trips intact instead of truncating" \
  "semi;colon.txt"

# Exact-byte assertion on the encoder itself, independent of the full completions pipeline:
# a trailing-newline regression (from piping through `echo`) would otherwise only surface as
# a decode-side mismatch, several layers removed from the actual bug.
actual_hex="$(warp_hex_encode_string "checkout")"
if [[ "$actual_hex" == "636865636b6f7574" ]]; then
  echo "PASS: warp_hex_encode_string produces exactly the expected bytes, with no trailing newline"
else
  echo "FAIL: warp_hex_encode_string produces exactly the expected bytes, with no trailing newline"
  echo "  expected: 636865636b6f7574"
  echo "  actual:   $actual_hex"
  failures=$((failures + 1))
fi

collect_replies "dashn-cli -"
assert_reply "a candidate that is exactly '-n' round-trips intact instead of encoding to nothing" "-n"

collect_replies "nestprobe-cli c"
assert_reply "a nested-paren description doesn't fold part of itself into the name" "cmd" "cmdx"
assert_descriptions "a nested-paren description keeps its own inner parens and spacing intact" \
  "outer  (inner) tail" "simple"

collect_replies "parenfile-cli B"
assert_reply "a lone real candidate that merely looks padded is not split" "Backup  (copy)"

if [[ $failures -eq 0 ]]; then
  echo "All bash native completions tests passed."
  exit 0
else
  echo "$failures test(s) failed."
  exit 1
fi
