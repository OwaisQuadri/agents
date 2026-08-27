#!/bin/zsh
set -euo pipefail
node --input-type=module -e 'import { responseFor } from "./api.mjs"; console.log(JSON.stringify([responseFor({role:"admin",name:" ADA "}),responseFor({role:"customer",name:" GRACE "})]));'
