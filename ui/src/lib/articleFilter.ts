import type { Article } from '../types'

// 一覧の表示判定。表示中に解除した行は次の再取得まで手元に残るため、
// 現在の favorite 値でも絞る。
export function isArticleVisible(
  article: Article,
  showUnreadOnly: boolean,
  readIds: Set<string>,
  showFavoritesOnly: boolean,
): boolean {
  return (
    (!showUnreadOnly || !readIds.has(article.id)) && (!showFavoritesOnly || article.favorite)
  )
}
