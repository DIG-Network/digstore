#!/usr/bin/env python3
"""Render the Digstore whitepaper Markdown to a print-quality PDF.

Pipeline: Markdown -> styled HTML -> Chrome/Edge headless --print-to-pdf.
No LaTeX/pandoc needed; uses the Chromium print engine already on the machine.

Usage:
    python render.py digstore-whitepaper.md digstore-whitepaper.pdf
"""
import sys
import os
import shutil
import subprocess
import tempfile

import markdown

CSS = r"""
@page { size: Letter; margin: 22mm 20mm 20mm 20mm; }
:root { --ink:#1a1a1a; --muted:#555; --rule:#d0d0d0; --accent:#243b53; --code-bg:#f6f8fa; }
* { box-sizing: border-box; }
html { -webkit-print-color-adjust: exact; print-color-adjust: exact; }
body {
  font-family: "Charter", "Georgia", "Times New Roman", serif;
  color: var(--ink); font-size: 10.5pt; line-height: 1.5; margin: 0;
}
h1, h2, h3, h4 { font-family: "Segoe UI", "Helvetica Neue", Arial, sans-serif; color: var(--accent); line-height: 1.25; }
h1 { font-size: 22pt; margin: 0 0 4pt; }
h2 { font-size: 15pt; margin: 20pt 0 6pt; padding-bottom: 3pt; border-bottom: 1.5pt solid var(--accent); page-break-after: avoid; }
h3 { font-size: 12pt; margin: 14pt 0 4pt; page-break-after: avoid; }
h4 { font-size: 10.5pt; margin: 10pt 0 3pt; color: var(--muted); page-break-after: avoid; }
p, li { orphans: 2; widows: 2; }
a { color: var(--accent); text-decoration: none; }
code, pre { font-family: "Cascadia Code", "Consolas", "SFMono-Regular", monospace; font-size: 8.8pt; }
code { background: var(--code-bg); padding: 0.5pt 3pt; border-radius: 3px; }
pre { background: var(--code-bg); border: 0.5pt solid var(--rule); border-radius: 5px; padding: 8pt 10pt; overflow-x: auto; page-break-inside: avoid; }
pre code { background: none; padding: 0; }
table { border-collapse: collapse; width: 100%; margin: 8pt 0; font-size: 9pt; page-break-inside: avoid; }
th, td { border: 0.5pt solid var(--rule); padding: 4pt 7pt; text-align: left; vertical-align: top; }
th { background: #eef2f7; font-family: "Segoe UI", Arial, sans-serif; font-weight: 600; }
blockquote { border-left: 3pt solid var(--accent); margin: 8pt 0; padding: 2pt 12pt; color: var(--muted); background: #fafbfc; }
hr { border: none; border-top: 0.5pt solid var(--rule); margin: 16pt 0; }
.title-block { text-align: center; margin: 40pt 0 28pt; }
.title-block .subtitle { font-size: 13pt; color: var(--muted); font-family: "Segoe UI", Arial, sans-serif; }
.title-block .meta { font-size: 10pt; color: var(--muted); margin-top: 10pt; }
.callout { border: 0.5pt solid #c8d3e0; background: #f4f8fd; border-radius: 5px; padding: 7pt 11pt; margin: 9pt 0; page-break-inside: avoid; }
h2, h3 { string-set: none; }
"""

TEMPLATE = """<!doctype html>
<html lang="en"><head><meta charset="utf-8"><style>{css}</style></head>
<body>{body}</body></html>
"""


def find_chrome():
    candidates = [
        os.path.expandvars(r"%ProgramFiles%\Google\Chrome\Application\chrome.exe"),
        os.path.expandvars(r"%ProgramFiles(x86)%\Google\Chrome\Application\chrome.exe"),
        os.path.expandvars(r"%ProgramFiles(x86)%\Microsoft\Edge\Application\msedge.exe"),
        os.path.expandvars(r"%ProgramFiles%\Microsoft\Edge\Application\msedge.exe"),
        shutil.which("chrome"),
        shutil.which("msedge"),
    ]
    for c in candidates:
        if c and os.path.exists(c):
            return c
    raise SystemExit("No Chrome/Edge found for PDF rendering.")


def main():
    md_path = sys.argv[1] if len(sys.argv) > 1 else "digstore-whitepaper.md"
    pdf_path = sys.argv[2] if len(sys.argv) > 2 else "digstore-whitepaper.pdf"
    with open(md_path, "r", encoding="utf-8") as f:
        text = f.read()
    html_body = markdown.markdown(
        text,
        extensions=["tables", "fenced_code", "toc", "codehilite", "sane_lists", "attr_list"],
        extension_configs={"codehilite": {"guess_lang": False, "noclasses": True}},
    )
    html = TEMPLATE.format(css=CSS, body=html_body)
    html_path = os.path.splitext(pdf_path)[0] + ".html"
    with open(html_path, "w", encoding="utf-8") as f:
        f.write(html)

    chrome = find_chrome()
    out_abs = os.path.abspath(pdf_path)
    url = "file:///" + os.path.abspath(html_path).replace("\\", "/")
    with tempfile.TemporaryDirectory() as profile:
        cmd = [
            chrome, "--headless", "--disable-gpu", "--no-sandbox",
            f"--user-data-dir={profile}",
            "--no-pdf-header-footer",
            f"--print-to-pdf={out_abs}",
            url,
        ]
        subprocess.run(cmd, check=True, timeout=120)
    print(f"Wrote {out_abs} ({os.path.getsize(out_abs)} bytes) via {os.path.basename(chrome)}")


if __name__ == "__main__":
    main()
