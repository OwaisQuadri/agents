export function total(values) {
  return values.reduce((state, value) => ({
    history: [...state.history, value],
    total: state.total + value,
  }), { history: [], total: 0 }).total;
}
