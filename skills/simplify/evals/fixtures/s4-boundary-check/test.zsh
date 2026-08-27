#!/bin/zsh
set -euo pipefail
node --input-type=module -e 'import { decodeRequest } from "./decoder.mjs"; console.log(decodeRequest("{\"name\":\"Ada\"}").name); try { decodeRequest("{}"); process.exit(2); } catch (error) { console.log(error.message); }'
