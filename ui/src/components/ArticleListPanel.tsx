import { Article } from '../types'
import { ArticleListItem } from './ArticleListItem'

interface ArticleListPanelProps {
  articles: Article[]
  loading: boolean
  showUnreadOnly: boolean
  showFavoritesOnly: boolean
  selectedArticle: Article | null
  onToggleUnread: () => void
  onToggleFavorites: () => void
  onSelectArticle: (article: Article) => void
  onToggleFavorite: (article: Article) => void
  onMarkAllRead: () => void
  onShowSidebar: () => void
}

export function ArticleListPanel({
  articles,
  loading,
  showUnreadOnly,
  showFavoritesOnly,
  selectedArticle,
  onToggleUnread,
  onToggleFavorites,
  onSelectArticle,
  onToggleFavorite,
  onMarkAllRead,
  onShowSidebar,
}: ArticleListPanelProps) {
  return (
    <div className="list-inner">
      <div className="list-toolbar">
        <button className="btn-icon mobile-menu-btn" onClick={onShowSidebar} title="メニュー">
          ☰
        </button>
        <div className="toolbar-controls">
          <label className="toggle-label">
            <input
              type="checkbox"
              checked={showUnreadOnly}
              onChange={onToggleUnread}
            />
            未読のみ
          </label>
          <label className="toggle-label">
            <input
              type="checkbox"
              checked={showFavoritesOnly}
              onChange={onToggleFavorites}
            />
            お気に入りのみ
          </label>
          <button className="btn btn--secondary btn--sm" onClick={onMarkAllRead}>
            全既読
          </button>
        </div>
      </div>

      {loading ? (
        <div className="list-loading">読み込み中...</div>
      ) : articles.length === 0 ? (
        <div className="list-empty">記事がありません</div>
      ) : (
        <ul className="article-list-items">
          {articles.map((article) => (
            <ArticleListItem
              key={article.id}
              article={article}
              isSelected={selectedArticle?.id === article.id}
              onClick={() => onSelectArticle(article)}
              onToggleFavorite={onToggleFavorite}
            />
          ))}
        </ul>
      )}
    </div>
  )
}
