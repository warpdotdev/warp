'''
Generates the match statement used in `app/src/font_fallback.rs` to map Unicode
code points to fallback fonts. Should be used in tandem with `generate-families.py`.
Generated code is sent to stdout.

This script will read our cloud storage bucket to download the font data for each
fallback font that we support. A directory "downloaded_fonts" will be created in
the directory where the script is executed to contain the downloaded fonts. If
that directory already exists, it is assumed that the fonts have previously been
downloaded and skips downloading them again.

Assumptions:
- The fallback fonts in the prod `warp-static-assets` bucket are identical to the
  ones stored in the staging `warp-server-staging-static-assets` bucket.
- For each font family in the bucket, there is a variant that contains "Regular"
  in the filename.

Usage:
1. Install the dependencies in `requirements.txt`.
2. Make sure the gcloud CLI is installed and you are authed via `gcloud auth login`.
3. Make sure you're running the script from `scripts/font_fallback`.
4. Run `python3 generate-mappings.py`.
'''

import os
import subprocess
import sys
from operator import itemgetter
from fontTools.ttLib import TTFont
from collections import defaultdict

# Represents priority for each font, with a lower value being higher priority.
# Since a code point can be supported by multiple fonts, the script will map the
# code point to the font with more priority.
GLOBAL_ORDER = {
    "Hack Nerd Font": 1,
    "Noto Color Emoji": 2,
    "Noto Sans Symbols": 3,
    "Noto Sans Symbols 2": 4,
    "Noto Sans SC": 5,
    "Noto Sans JP": 6,
    "Noto Sans Devanagari": 7,
}

# By default, we try to coalesce code point ranges. e.g. If we have the
# following mappings:
#   U+10000..U+10002 -> Noto Sans SC
#   U+10004..U+10005 -> Noto Sans SC
# If U+10003 is not represented by any font, we will merge these code points into
# a single range even though there is an unsupported code point in the range:
#   U+10000..U+10005 -> Noto Sans SC
# This is done to reduce the total number of mappings and is useful for handling
# Unicode blocks with rare characters where font support for these characters is
# sparse.
#
# However, we don't want to do this for some fonts where unsupported code points
# could represent icons or private use areas in Unicode. Those fonts are added
# to this set.
FONTS_NOT_TO_COALESCE = set([
    "Noto Sans Symbols",
    "Noto Sans Symbols 2",
    "Noto Color Emoji",
    "Hack Nerd Font"
])

HACK_FONT_FILEPATH = "../../app/assets/bundled/fonts/hack/Hack-Regular.ttf"
ROBOTO_FONT_FILEPATH = "../../app/assets/bundled/fonts/roboto/Roboto-Regular.ttf"
FONT_DOWNLOAD_DIR = "./downloaded_fonts"


def download_fallback_fonts():
    if os.path.exists(FONT_DOWNLOAD_DIR):
        # Fonts already exist, no need to download
        return

    os.mkdir(FONT_DOWNLOAD_DIR)
    command = ["gcloud", "storage", "cp", "gs://warp-static-assets/fallback-fonts/**/*Regular*.ttf", FONT_DOWNLOAD_DIR]
    return_code = subprocess.call(command)
    if return_code != 0:
        sys.exit("Failed to download fonts from GCP")


def get_global_order(font_name):
    if font_name in GLOBAL_ORDER:
        return GLOBAL_ORDER[font_name]
    else:
        return len(GLOBAL_ORDER) + 1


def get_default_fonts():
    return [TTFont(HACK_FONT_FILEPATH), TTFont(ROBOTO_FONT_FILEPATH)]


# Returns a `TTFont` object for each fallback font, sorted by their global order.
def get_fallback_fonts(fallback_fonts_dir):
    fonts = []
    for file in os.listdir(fallback_fonts_dir):
        if file.endswith(".ttf"):
            path = os.path.join(fallback_fonts_dir, file)
            font = TTFont(path)

            font_name = get_font_name(font)
            global_order = get_global_order(font_name)
            fonts.append((font, global_order))

    fonts.sort(key=itemgetter(1))
    return [font for (font, _) in fonts]


def get_font_name(font):
    return font['name'].getBestFamilyName()


def supported_code_points(font):
    code_points = set()
    for table in font['cmap'].tables:
        if table.isUnicode():
            code_points.update(table.cmap.keys())
    return code_points


def common_code_points(fonts):
    code_points = [supported_code_points(font) for font in fonts]
    return set.intersection(*code_points)


def generate_mapping(default_fonts, fallback_fonts):
    default_code_points = common_code_points(default_fonts)
    mapping = {}
    for font in reversed(fallback_fonts):
        font_name = get_font_name(font)
        font_code_points = supported_code_points(font)
        for code_point in font_code_points:
            if code_point in default_code_points:
                continue
            mapping[code_point] = font_name
    ranges = coalesce_ranges(collapse_to_ranges(mapping))
    font_ranges_map = collect_ranges_to_map(ranges)
    print_match_statement(font_ranges_map)


# Takes a mapping of individual code points -> fallback fonts and merges them
# into ranges where consecutive code points map to the same fallback font.
def collapse_to_ranges(mapping):
    mapping_list = [(k, v) for k, v in mapping.items()]
    mapping_list.sort(key=itemgetter(0))

    ranges = []
    prev_font = None
    active_range = None

    for code_point, font in mapping_list:
        if active_range and prev_font and (active_range[1], prev_font) != (code_point - 1, font):
            ranges.append((active_range, prev_font))
            active_range = None

        if not active_range:
            active_range = (code_point, code_point)
        else:
            active_range = (active_range[0], code_point)
        prev_font = font

    if active_range and prev_font:
        ranges.append((active_range, prev_font))

    return ranges


# Takes a mapping of code point ranges -> fallback fonts and coalesces them. See
# the comment on `FONTS_NOT_TO_COALESCE` for more details on coalescing.
def coalesce_ranges(ranges):
    new_ranges = []
    prev_font = None
    active_range = None

    for cur_range, font in ranges:
        if font in FONTS_NOT_TO_COALESCE:
            if active_range and prev_font:
                new_ranges.append((active_range, prev_font))
            new_ranges.append((cur_range, font))
            prev_font = None
            active_range = None
            continue

        if active_range and prev_font and prev_font != font:
            new_ranges.append((active_range, prev_font))
            active_range = None

        if not active_range:
            active_range = cur_range
        else:
            active_range = (active_range[0], cur_range[1])
        prev_font = font

    if active_range and prev_font:
        ranges.append((active_range, prev_font))

    return new_ranges


# Collects the mappings into a dictionary where each font is mapped to a list of
# all the code point ranges that it supports.
def collect_ranges_to_map(ranges):
    font_ranges_map = defaultdict(list)
    for cur_range, font in ranges:
        font_ranges_map[font].append(cur_range)
    return font_ranges_map


def match_arm(code_point_range, font_name):
    range_start, range_end = code_point_range
    constant_case_font_name = font_name.replace(" ", "_").upper()

    match_start = f"\\u{{{range_start:04X}}}"
    match_end = f"\\u{{{range_end:04X}}}"
    font_family = f"Some({constant_case_font_name}.clone())"
    return f"'{match_start}'..='{match_end}' => {font_family},"


def print_font_ranges(font_name, ranges):
    constant_case_font_name = font_name.replace(" ", "_").upper()
    font_family = f"Some({constant_case_font_name}.clone())"
    for i, (range_start, range_end) in enumerate(ranges):
        match_start = f"\\u{{{range_start:04X}}}"
        match_end = f"\\u{{{range_end:04X}}}"
        line = ""
        if i > 0:
            line += "| "
        line += f"'{match_start}'..='{match_end}'"
        if i == len(ranges) - 1:
            line += f" => {font_family},"
        print(line)


def print_match_statement(font_ranges_map):
    print("match ch {")

    for font_name, ranges in font_ranges_map.items():
        print_font_ranges(font_name, ranges)

    print("_ => None")
    print("}")


def main():
    default_fonts = get_default_fonts()

    download_fallback_fonts()
    fallback_fonts = get_fallback_fonts(FONT_DOWNLOAD_DIR)

    generate_mapping(default_fonts, fallback_fonts)


if __name__ == "__main__":
    main()
