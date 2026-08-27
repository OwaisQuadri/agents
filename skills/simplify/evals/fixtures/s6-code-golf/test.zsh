#!/bin/zsh
set -euo pipefail
node --input-type=module -e 'import { statusLabel } from "./status.mjs"; const rows = [{isActive:false,isAdmin:false,isTrial:false},{isActive:true,isAdmin:true,isTrial:false},{isActive:true,isAdmin:false,isTrial:true},{isActive:true,isAdmin:false,isTrial:false}]; console.log(rows.map(statusLabel).join(","));'
