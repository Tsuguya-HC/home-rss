import { useState, useEffect, useCallback, useRef } from 'react'
import { Feed, Article } from './types'
import { api } from './api'

type MobileView = 'sidebar' | 'list' | 'detail'

export default function App() {
  const [feeds, setFeeds] = useState<Feed[]>([])
  const [articles, setArticles] = useState<Article[]>([])
  const [unreadCounts, setUnreadCounts] = useState<Record<string, number>>({})
  const [selectedFeedId, setSelectedFeedId] = useState<string | null>(null)
  const [selectedArticle, setSelectedArticle] = useState<Article | null>(null)
  const [showUnreadOnly, setShowUnreadOnly] = useState(true)
  const [readIds, setReadIds] = useState<Set<string>>(new Set())
  const [mobileView, setMobileView] = useState<MobileView>('list')
  const [error, setError] = useState<string | null>(null)
  const [loadingArticles, setLoadingArticles] = useState(false)

  const withError = async (fn: () => Promise<void>) => {
    try {
      setError(null)
      await fn()
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e))
    }
  }

  const loadUnreadCounts = useCallback(async () => {
    const unread = await api.getArticles(null, true)
    const counts: Record<string, number> = {}
    for (const a of unread) {
      counts[a.feed_id] = (counts[a.feed_id] || 0) + 1
    }
    setUnreadCounts(counts)
  }, [])

  const loadFeeds = useCallback(async () => {
    const [feeds] = await Promise.all([api.getFeeds(), loadUnreadCounts()])
    setFeeds(feeds)
  }, [loadUnreadCounts])

  const loadArticles = useCallback(async () => {
    setLoadingArticles(true)
    try {
      const articles = await api.getArticles(selectedFeedId, showUnreadOnly)
      setArticles(articles)
      setReadIds(new Set())
    } finally {
      setLoadingArticles(false)
    }
  }, [selectedFeedId, showUnreadOnly])

  useEffect(() => {
    withError(loadFeeds)
  }, [loadFeeds])

  useEffect(() => {
    withError(loadArticles)
  }, [loadArticles])

  const handleSelectFeed = (feedId: string | null) => {
    setSelectedFeedId(feedId)
    setSelectedArticle(null)
    setMobileView('list')
  }

  const handleSelectArticle = async (article: Article) => {
    setSelectedArticle(article)
    setMobileView('detail')
    await withError(async () => {
      await api.markRead(article.id)
      setReadIds((prev) => new Set([...prev, article.id]))
      setUnreadCounts((prev) => ({
        ...prev,
        [article.feed_id]: Math.max(0, (prev[article.feed_id] || 0) - 1),
      }))
    })
  }

  const handleMarkAllRead = () =>
    withError(async () => {
      await api.markAllRead()
      setReadIds(new Set(articles.map((a) => a.id)))
      setUnreadCounts({})
    })

  const handleAddFeed = async (url: string) => {
    await withError(async () => {
      await api.addFeed(url)
      await loadFeeds()
    })
  }

  const handleDeleteFeed = async (id: string) => {
    await withError(async () => {
      await api.deleteFeed(id)
      if (selectedFeedId === id) {
        setSelectedFeedId(null)
        setSelectedArticle(null)
      }
      await loadFeeds()
      await loadArticles()
    })
  }

  const handleImportOpml = async (file: File) => {
    await withError(async () => {
      await api.importOpml(file)
      await loadFeeds()
    })
  }

  const visibleArticles = showUnreadOnly
    ? articles.filter((a) => !readIds.has(a.id))
    : articles

  const totalUnread = Object.values(unreadCounts).reduce((a, b) => a + b, 0)

  return (
    <div className="app">
      {error && (
        <div className="error-banner" onClick={() => setError(null)}>
          {error} <span className="error-close">×</span>
        </div>
      )}
      <div className="layout">
        <aside className={`sidebar${mobileView !== 'sidebar' ? ' sidebar--hidden' : ''}`}>
          <Sidebar
            feeds={feeds}
            unreadCounts={unreadCounts}
            totalUnread={totalUnread}
            selectedFeedId={selectedFeedId}
            onSelectFeed={handleSelectFeed}
            onAddFeed={handleAddFeed}
            onDeleteFeed={handleDeleteFeed}
            onImportOpml={handleImportOpml}
          />
        </aside>

        <section className={`article-list${mobileView === 'detail' ? ' article-list--hidden' : ''}`}>
          <ArticleListPanel
            articles={visibleArticles}
            loading={loadingArticles}
            showUnreadOnly={showUnreadOnly}
            selectedArticle={selectedArticle}
            onToggleUnread={() => {
              setShowUnreadOnly((v) => !v)
              setSelectedArticle(null)
            }}
            onSelectArticle={handleSelectArticle}
            onMarkAllRead={handleMarkAllRead}
            onShowSidebar={() => setMobileView('sidebar')}
          />
        </section>

        <section className={`article-detail${mobileView !== 'detail' ? ' article-detail--hidden' : ''}`}>
          {selectedArticle ? (
            <ArticleDetail
              article={selectedArticle}
              onBack={() => setMobileView('list')}
            />
          ) : (
            <div className="detail-placeholder">記事を選択してください</div>
          )}
        </section>
      </div>
    </div>
  )
}

// --- Sidebar ---

interface SidebarProps {
  feeds: Feed[]
  unreadCounts: Record<string, number>
  totalUnread: number
  selectedFeedId: string | null
  onSelectFeed: (id: string | null) => void
  onAddFeed: (url: string) => Promise<void>
  onDeleteFeed: (id: string) => Promise<void>
  onImportOpml: (file: File) => Promise<void>
}

function Sidebar({
  feeds,
  unreadCounts,
  totalUnread,
  selectedFeedId,
  onSelectFeed,
  onAddFeed,
  onDeleteFeed,
  onImportOpml,
}: SidebarProps) {
  const [showAddFeed, setShowAddFeed] = useState(false)
  const fileInputRef = useRef<HTMLInputElement>(null)

  const handleOpmlChange = async (e: React.ChangeEvent<HTMLInputElement>) => {
    const file = e.target.files?.[0]
    if (!file) return
    await onImportOpml(file)
    e.target.value = ''
  }

  return (
    <div className="sidebar-inner">
      <div className="sidebar-header">
        <h1 className="app-title">home-rss</h1>
      </div>

      <nav className="feed-list">
        <button
          className={`feed-item${selectedFeedId === null ? ' feed-item--active' : ''}`}
          onClick={() => onSelectFeed(null)}
        >
          <span className="feed-name">すべて</span>
          {totalUnread > 0 && (
            <span className="unread-badge">{totalUnread}</span>
          )}
        </button>

        {feeds.map((feed) => (
          <FeedItem
            key={feed.id}
            feed={feed}
            unreadCount={unreadCounts[feed.id] || 0}
            isSelected={selectedFeedId === feed.id}
            onSelect={() => onSelectFeed(feed.id)}
            onDelete={() => onDeleteFeed(feed.id)}
          />
        ))}
      </nav>

      <div className="sidebar-actions">
        <button className="btn btn--primary btn--full" onClick={() => setShowAddFeed(true)}>
          + フィード追加
        </button>
        <button
          className="btn btn--secondary btn--full"
          onClick={() => fileInputRef.current?.click()}
        >
          OPML インポート
        </button>
        <input
          ref={fileInputRef}
          type="file"
          accept=".opml,.xml"
          style={{ display: 'none' }}
          onChange={handleOpmlChange}
        />
      </div>

      {showAddFeed && (
        <AddFeedModal
          onAdd={async (url) => {
            await onAddFeed(url)
            setShowAddFeed(false)
          }}
          onClose={() => setShowAddFeed(false)}
        />
      )}
    </div>
  )
}

// --- FeedItem ---

interface FeedItemProps {
  feed: Feed
  unreadCount: number
  isSelected: boolean
  onSelect: () => void
  onDelete: () => Promise<void>
}

function FeedItem({ feed, unreadCount, isSelected, onSelect, onDelete }: FeedItemProps) {
  const [confirmDelete, setConfirmDelete] = useState(false)

  const handleDelete = async (e: React.MouseEvent) => {
    e.stopPropagation()
    if (!confirmDelete) {
      setConfirmDelete(true)
      setTimeout(() => setConfirmDelete(false), 3000)
      return
    }
    await onDelete()
  }

  return (
    <div className={`feed-item${isSelected ? ' feed-item--active' : ''}`} onClick={onSelect}>
      <span className="feed-name">{feed.title || feed.url}</span>
      <span className="feed-actions">
        {unreadCount > 0 && <span className="unread-badge">{unreadCount}</span>}
        <button
          className={`delete-btn${confirmDelete ? ' delete-btn--confirm' : ''}`}
          onClick={handleDelete}
          title={confirmDelete ? '確認: もう一度クリックで削除' : '削除'}
        >
          {confirmDelete ? '!' : '×'}
        </button>
      </span>
    </div>
  )
}

// --- AddFeedModal ---

interface AddFeedModalProps {
  onAdd: (url: string) => Promise<void>
  onClose: () => void
}

function AddFeedModal({ onAdd, onClose }: AddFeedModalProps) {
  const [url, setUrl] = useState('')
  const [loading, setLoading] = useState(false)

  const handleSubmit = async (e: React.FormEvent) => {
    e.preventDefault()
    if (!url.trim()) return
    setLoading(true)
    try {
      await onAdd(url.trim())
    } finally {
      setLoading(false)
    }
  }

  return (
    <div className="modal-overlay" onClick={onClose}>
      <div className="modal" onClick={(e) => e.stopPropagation()}>
        <h2 className="modal-title">フィード追加</h2>
        <form onSubmit={handleSubmit}>
          <input
            className="modal-input"
            type="url"
            placeholder="https://example.com/feed.xml"
            value={url}
            onChange={(e) => setUrl(e.target.value)}
            autoFocus
            required
          />
          <div className="modal-actions">
            <button type="button" className="btn btn--secondary" onClick={onClose}>
              キャンセル
            </button>
            <button type="submit" className="btn btn--primary" disabled={loading}>
              {loading ? '追加中...' : '追加'}
            </button>
          </div>
        </form>
      </div>
    </div>
  )
}

// --- ArticleListPanel ---

interface ArticleListPanelProps {
  articles: Article[]
  loading: boolean
  showUnreadOnly: boolean
  selectedArticle: Article | null
  onToggleUnread: () => void
  onSelectArticle: (article: Article) => void
  onMarkAllRead: () => void
  onShowSidebar: () => void
}

function ArticleListPanel({
  articles,
  loading,
  showUnreadOnly,
  selectedArticle,
  onToggleUnread,
  onSelectArticle,
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
            />
          ))}
        </ul>
      )}
    </div>
  )
}

// --- ArticleListItem ---

interface ArticleListItemProps {
  article: Article
  isSelected: boolean
  onClick: () => void
}

function ArticleListItem({ article, isSelected, onClick }: ArticleListItemProps) {
  const date = article.published_at
    ? new Date(article.published_at * 1000).toLocaleDateString('ja-JP', {
        month: 'short',
        day: 'numeric',
      })
    : null

  return (
    <li
      className={`article-item${isSelected ? ' article-item--active' : ''}`}
      onClick={onClick}
    >
      <div className="article-item-title">{article.title}</div>
      {date && <div className="article-item-date">{date}</div>}
    </li>
  )
}

// --- ArticleDetail ---

interface ArticleDetailProps {
  article: Article
  onBack: () => void
}

function ArticleDetail({ article, onBack }: ArticleDetailProps) {
  const date = article.published_at
    ? new Date(article.published_at * 1000).toLocaleString('ja-JP', {
        year: 'numeric',
        month: 'short',
        day: 'numeric',
        hour: '2-digit',
        minute: '2-digit',
      })
    : null

  return (
    <div className="detail-inner">
      <div className="detail-header">
        <button className="btn-icon back-btn mobile-back-btn" onClick={onBack} title="戻る">
          ←
        </button>
        <a
          href={article.url}
          target="_blank"
          rel="noopener noreferrer"
          className="detail-link"
        >
          元記事を開く ↗
        </a>
      </div>
      <article className="detail-article">
        <h1 className="detail-title">{article.title}</h1>
        <div className="detail-meta">
          {article.author && <span>{article.author}</span>}
          {date && <span>{date}</span>}
        </div>
        {article.content ? (
          <div
            className="detail-content"
            dangerouslySetInnerHTML={{ __html: article.content }}
          />
        ) : (
          <div className="detail-no-content">
            <p>本文がありません。</p>
            <a href={article.url} target="_blank" rel="noopener noreferrer">
              元記事を読む →
            </a>
          </div>
        )}
      </article>
    </div>
  )
}
