# docstring style

A docstring documents a declaration that callers outside the module use. The whitelist in
docs/comment-style.md decides whether it ships at all. This file decides its shape.

Every docstring carries four facts, and it carries no more than these.

1. what the declaration does, in one summary line.
2. its inputs.
3. its output.
4. its errors.

A docstring never explains the body. Where the body needs an explanation, the code is the
bug: rename, extract, and restructure it instead.

## the standard per language

Use the language's own documentation generator, and use that generator's tag vocabulary.
Never invent a house format on top of one. Where a generator's own spec disagrees with the
table below, the spec wins.

| language | standard | marker | inputs, output, errors |
| --- | --- | --- | --- |
| Python | Google style, over PEP 257 | `"""` | `Args:` · `Returns:` or `Yields:` · `Raises:` |
| Swift | DocC markup | `///` | `- Parameter <name>:` · `- Returns:` · `- Throws:` |
| Rust | rustdoc, per RFC 505 and RFC 1574 | `///` | prose · prose · `# Errors`, `# Panics`, `# Safety` |
| Go | Go Doc Comments | `//` | prose only |
| TypeScript | TSDoc | `/** */` | `@param` · `@returns` · `@throws` |
| JavaScript | JSDoc | `/** */` | `@param` · `@returns` · `@throws` |
| Java | Javadoc | `/** */` | `@param` · `@return` · `@throws` |
| Kotlin | KDoc, rendered by Dokka | `/** */` | `@param` · `@return` · `@throws` |
| C# | XML documentation comments | `///` | `<param>` · `<returns>` · `<exception>` |
| C and C++ | Doxygen | `/** */` | `@param` · `@return` · `@throws` |
| Objective-C | Doxygen markup, which Xcode Quick Help reads | `/** */` | `@param` · `@return` |
| PHP | PHPDoc | `/** */` | `@param` · `@return` · `@throws` |
| Ruby | RDoc | `#` | prose only |
| Lua | LuaCATS annotations, read by lua-language-server | `---` | `---@param` · `---@return` |
| Zig | doc comments, read by autodoc | `///` | prose only |
| Elixir | `@doc` and `@moduledoc`, rendered by ExDoc | `@doc """` | prose only |

Three rows carry a caveat.

- Go wants a full sentence, and it wants that sentence to open with the identifier's own
  name. Go has no tag vocabulary.
- Rust wants the summary line in third person singular present. Its parameters and its
  return value go in prose, and only the error cases get their own headings.
- Ruby ships RDoc, and much of its ecosystem runs YARD instead. Use YARD's `@param`,
  `@return`, and `@raise` where the project already runs YARD. Use RDoc prose everywhere
  else.

## a language this file does not list

Two questions settle it, in order.

1. does the project already run a documentation generator? Follow it, and stop here.
2. does the language ship or bless one? Use it, and add its row above.

Where neither question answers, the language has no inline documentation to generate. Write
no docstring there. The whitelist in docs/comment-style.md governs every other comment.
