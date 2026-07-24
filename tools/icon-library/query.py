"""
Query the icon library using Claude to find the best icons for a use case.

Usage:
    uv run query.py "a button to save a file"
    uv run query.py "navigation between app sections"
    uv run query.py --list        # list all icons in DB
    uv run query.py --svg <name>  # print raw SVG for an icon
"""

import sys
import json
import os
import anthropic
import db

MODEL = "claude-sonnet-4-6"

SYSTEM_PROMPT = """You are an expert UI/UX designer helping developers choose the right icon for a use case.

You will be given:
1. A catalog of available icons (name, tags, description)
2. A use case description from the developer

Your job is to reason about which icons best match the use case and return 1–5 recommendations ranked from best to worst match.

Guidelines:
- Consider the semantic meaning of icon names and tags carefully
- Think about common UI conventions and what users expect
- Prefer icons that clearly communicate the intended action/concept
- If multiple icons could work, include up to 5 in ranked order
- If no icon is a good match, say so honestly (return an empty list)
- Be concise in your reasoning — one sentence per icon is enough

You MUST respond with valid JSON only, in this exact format:
{
  "reasoning": "Brief overall reasoning about your selection approach",
  "recommendations": [
    {
      "name": "icon-name",
      "reason": "Why this icon fits the use case"
    }
  ]
}"""


def build_catalog_text(icons: list[dict]) -> str:
    lines = ["Available icons:"]
    for icon in icons:
        tags_str = ", ".join(icon["tags"]) if icon["tags"] else ""
        desc_str = f" — {icon['description']}" if icon.get("description") else ""
        lines.append(f"  • {icon['name']}  [tags: {tags_str}]{desc_str}")
    return "\n".join(lines)


def query_icons(use_case: str, verbose: bool = False) -> dict:
    icons = db.get_all_icons()
    if not icons:
        return {"error": "No icons in database. Run: uv run ingest.py <directory>"}

    catalog = build_catalog_text(icons)

    client = anthropic.Anthropic()

    response = client.messages.create(
        model=MODEL,
        max_tokens=1024,
        system=[
            {
                "type": "text",
                "text": SYSTEM_PROMPT,
                "cache_control": {"type": "ephemeral"},
            },
            {
                "type": "text",
                "text": catalog,
                "cache_control": {"type": "ephemeral"},
            },
        ],
        messages=[
            {
                "role": "user",
                "content": f"Use case: {use_case}",
            }
        ],
    )

    if verbose:
        usage = response.usage
        print(f"[tokens] input={usage.input_tokens} output={usage.output_tokens} "
              f"cache_read={getattr(usage, 'cache_read_input_tokens', 0)} "
              f"cache_created={getattr(usage, 'cache_creation_input_tokens', 0)}")

    raw = response.content[0].text.strip()
    # Strip markdown code fences if present
    if raw.startswith("```"):
        raw = raw.split("\n", 1)[1].rsplit("```", 1)[0].strip()

    return json.loads(raw)


def print_results(use_case: str, result: dict, show_svg: bool = True):
    if "error" in result:
        print(f"Error: {result['error']}")
        return

    print(f"\nUse case: {use_case}")
    print(f"Reasoning: {result.get('reasoning', '')}\n")

    recommendations = result.get("recommendations", [])
    if not recommendations:
        print("No matching icons found for this use case.")
        return

    print(f"Recommendations ({len(recommendations)}):")
    for i, rec in enumerate(recommendations, 1):
        name = rec["name"]
        reason = rec.get("reason", "")
        print(f"\n  {i}. {name}")
        print(f"     {reason}")
        if show_svg:
            content = db.get_icon_content(name)
            if content:
                print(f"\n     SVG:\n     {content.replace(chr(10), chr(10) + '     ')}")
            else:
                print(f"     (icon '{name}' not found in DB)")


def main():
    args = sys.argv[1:]

    if not args or "--help" in args or "-h" in args:
        print(__doc__)
        sys.exit(0)

    db.init_db()

    if "--list" in args:
        icons = db.get_all_icons()
        if not icons:
            print("No icons in database. Run: uv run ingest.py <directory>")
        else:
            print(f"{len(icons)} icons in database:\n")
            for icon in icons:
                tags = ", ".join(icon["tags"][:4])
                print(f"  {icon['name']:<30} tags: {tags}")
        return

    if "--svg" in args:
        idx = args.index("--svg")
        if idx + 1 >= len(args):
            print("Usage: uv run query.py --svg <name>")
            sys.exit(1)
        name = args[idx + 1]
        content = db.get_icon_content(name)
        if content:
            print(content)
        else:
            print(f"Icon '{name}' not found. Run --list to see all icons.")
        return

    verbose = "--verbose" in args or "-v" in args
    no_svg = "--no-svg" in args
    use_case_parts = [a for a in args if not a.startswith("-")]
    use_case = " ".join(use_case_parts)

    if not use_case:
        print("Please provide a use case description.")
        sys.exit(1)

    if not os.environ.get("ANTHROPIC_API_KEY"):
        print("Error: ANTHROPIC_API_KEY environment variable not set.")
        sys.exit(1)

    result = query_icons(use_case, verbose=verbose)
    print_results(use_case, result, show_svg=not no_svg)


if __name__ == "__main__":
    main()
