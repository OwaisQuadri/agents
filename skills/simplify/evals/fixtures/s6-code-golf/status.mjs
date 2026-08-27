export function statusLabel(account) {
  if (!account.isActive) {
    return "inactive";
  }
  if (account.isAdmin) {
    return "administrator";
  }
  if (account.isTrial) {
    return "trial";
  }
  return "member";
}
