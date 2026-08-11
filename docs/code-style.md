# code style — manual overrides

Rules the user has set by hand. They override default style judgment and
language-convention instinct; where a rule here conflicts with either, this file wins.
One short rule per line, added as a "- " bullet.

- booleans are always named with an `is` prefix
- **UI descriptions:** Do not add subtitles, helper text, or descriptive copy beneath headings, labels, cards, or settings by default. Prefer one concise, self-explanatory heading or label. Only add supporting copy when the user explicitly asks for it or when it is necessary to prevent misunderstanding or error, and never use it to restate the heading.
- **Docstrings:** a docstring goes on a declaration that callers outside the module use, and nowhere else. It carries four facts: what the declaration does, its inputs, its output, and its errors. Follow the language's own documentation generator and its tag vocabulary. The file docs/docstring-style.md names the standard per language. The whitelist in docs/comment-style.md owns whether a docstring ships at all.
