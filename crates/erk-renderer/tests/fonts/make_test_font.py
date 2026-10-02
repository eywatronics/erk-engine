"""Generates ErkTest.ttf, the font the system-font tests serve as a host would.

Every glyph is a square 0.8 em wide on a 1 em advance, so a test can tell from
the advances alone which font drew a character: Noto Sans has none of the
CJK, Hebrew or emoji glyphs, and its `x` is about half an em wide.

    python make_test_font.py

Needs fontTools. The output is committed; this script only documents how it
was made. The font is part of the project and under its licence.
"""

from pathlib import Path

from fontTools.fontBuilder import FontBuilder
from fontTools.pens.ttGlyphPen import TTGlyphPen

UNITS = 1000
CHARACTERS = {
    "x": ord("x"),
    "uni4E2D": 0x4E2D,  # 中
    "uni6587": 0x6587,  # 文
    "uni05D0": 0x05D0,  # א
    "u1F600": 0x1F600,  # 😀
}


def square():
    pen = TTGlyphPen(None)
    pen.moveTo((100, 0))
    pen.lineTo((100, 800))
    pen.lineTo((900, 800))
    pen.lineTo((900, 0))
    pen.closePath()
    return pen.glyph()


def main():
    names = [".notdef", *CHARACTERS]
    builder = FontBuilder(UNITS, isTTF=True)
    builder.setupGlyphOrder(names)
    builder.setupCharacterMap(dict((code, name) for name, code in CHARACTERS.items()))
    empty = TTGlyphPen(None).glyph()
    builder.setupGlyf({name: (empty if name == ".notdef" else square()) for name in names})
    builder.setupHorizontalMetrics({name: (UNITS, 0 if name == ".notdef" else 100) for name in names})
    builder.setupHorizontalHeader(ascent=800, descent=-200)
    builder.setupNameTable({"familyName": "Erk Test", "styleName": "Regular"})
    builder.setupOS2(
        sTypoAscender=800,
        sTypoDescender=-200,
        sTypoLineGap=0,
        usWinAscent=800,
        usWinDescent=200,
        usWeightClass=400,
    )
    builder.setupPost()
    builder.save(Path(__file__).with_name("ErkTest.ttf"))


if __name__ == "__main__":
    main()
