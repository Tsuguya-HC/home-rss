// 星ボタンの連打抑止。応答が返るまで同一記事の送信を 1 本に絞り、
// POST/DELETE の逆順確定を起こさない。state の更新は次の render まで
// 反映されないため、同 tick の連打を確実に弾く判定は呼び出し側の ref が担う。
export function beginFavoriteToggle(inflight: Set<string>, id: string): boolean {
  if (inflight.has(id)) return false
  inflight.add(id)
  return true
}

export function endFavoriteToggle(inflight: Set<string>, id: string): void {
  inflight.delete(id)
}
