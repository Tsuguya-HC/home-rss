import { useState, useEffect, useCallback } from 'react'
import { Feed, Article } from './types'
import { api } from './api'
import { Sidebar } from './components/Sidebar'
import { ArticleListPanel } from './components/ArticleListPanel'
import { ArticleDetail } from './components/ArticleDetail'

type MobileView = 'sidebar' | 'list' | 'detail'

export default function App() {
  const [feeds, setFeeds] = useState<Feed[]>([])
  const [articles, setArticles] = useState<Article[]>([])
  const [unreadCounts, setUnreadCounts] = useState<Record<string, number>>({})
  const [selectedFeedId, setSelectedFeedId] = useState<string | null>(null)
  const [selectedArticle, setSelectedArticle] = useState<Article | null>(null)
  const [showUnreadOnly, setShowUnreadOnly] = useState(true)
  const [showFavoritesOnly, setShowFavoritesOnly] = useState(false)
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

  const [favoriteOverrides, setFavoriteOverrides] = useState<Record<string, boolean>>({})

  const loadArticles = useCallback(async () => {
    setLoadingArticles(true)
    try {
      const articles = await api.getArticles(selectedFeedId, showUnreadOnly, showFavoritesOnly)
      setFavoriteOverrides((prev) => {
        if (Object.keys(prev).length === 0) return prev
        const fetched = new Map(articles.map((a) => [a.id, a.is_favorite] as const))
        let dropped = false
        const next: Record<string, boolean> = {}
        for (const [id, want] of Object.entries(prev)) {
          // 204 を受けた分だけが上書きとして残り、後の GET で裏付けの取れた分は
          // 落とす。一覧に無い記事の分は残る（お気に入りのみ表示で外れた記事や、
          // まだ裏付けの無い 204 未確定のトグル）。
          if (fetched.get(id) === want) dropped = true
          else next[id] = want
        }
        return dropped ? next : prev
      })
      setArticles(articles)
      setReadIds(new Set())
    } finally {
      setLoadingArticles(false)
    }
  }, [selectedFeedId, showUnreadOnly, showFavoritesOnly])

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

  const handleToggleFavorite = (article: Article) =>
    withError(async () => {
      const next = !(favoriteOverrides[article.id] ?? article.is_favorite)
      if (next) {
        await api.markFavorite(article.id)
      } else {
        await api.unmarkFavorite(article.id)
      }
      // 204 を受けたトグルのみ上書きに残し、選別中の後着の古い一覧で
      // 消されないようにする。既に処理済みの loadArticles が裏付けた分は
      // そこで落ちる。
      setFavoriteOverrides((prev) => ({ ...prev, [article.id]: next }))
    })

  const handleAddFeed = async (url: string) => {
    await withError(async () => {
      // 即時取得に失敗してもフィード行自体は DB に残るため、
      // エラー時も一覧を更新して「登録済み」が見えるようにする。
      let addFeedError: unknown = null
      try {
        await api.addFeed(url)
      } catch (e) {
        addFeedError = e
      }
      // loadFeeds/loadArticles は個別に catch し、片方が失敗してももう片方は
      // 必ず試みる（#106 R13）。addFeed・loadFeeds・loadArticles を別変数で
      // 持つのは、連結メッセージの前置きが実際に失敗した処理と食い違わない
      // ようにするため（#106 U11: 以前は loadFeeds の失敗が addFeed の失敗と
      // 同じ変数に入っており、追加自体は成功したのに失敗したかのような文面に
      // なっていた）。
      let loadFeedsError: unknown = null
      let loadArticlesError: unknown = null
      await loadFeeds().catch((e: unknown) => {
        loadFeedsError = e
      })
      await loadArticles().catch((e: unknown) => {
        loadArticlesError = e
      })

      const messageOf = (e: unknown) => (e instanceof Error ? e.message : String(e))

      // loadArticles はこの PR の核心（追加直後の記事をすぐ見せる）そのものな
      // ので、addFeed のエラーの陰に隠して不可視化しない。両方失敗した場合は
      // 連結して伝える（#106 R15）。
      if (addFeedError && loadArticlesError) {
        throw new Error(`${messageOf(addFeedError)}（記事一覧の再読み込みにも失敗しました）`)
      }
      if (addFeedError) throw addFeedError
      if (loadFeedsError && loadArticlesError) {
        throw new Error(
          `フィード一覧・記事一覧の再読み込みに失敗しました: ${messageOf(loadFeedsError)}`,
        )
      }
      if (loadFeedsError) throw loadFeedsError
      if (loadArticlesError) throw loadArticlesError
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
      const result = await api.importOpml(file)
      await loadFeeds()
      // ガードが入る前は OPML の全エントリが無条件でインポートされていた。
      // 弾かれた件数を無言で捨てると、内部ホスト向けや壊れたエントリが
      // エラーも件数も出ずに漏れてしまう (#106 U2)。imported + already_present
      // + skipped_invalid + skipped_blocked が入力件数と一致するように
      // already_present も併記する (#106 U8)。
      if (result.skipped_invalid > 0 || result.skipped_blocked > 0) {
        const reasons: string[] = []
        if (result.already_present > 0) {
          reasons.push(`${result.already_present}件は登録済みのため`)
        }
        if (result.skipped_invalid > 0) {
          reasons.push(`${result.skipped_invalid}件は不正なエントリのため`)
        }
        if (result.skipped_blocked > 0) {
          reasons.push(`${result.skipped_blocked}件は内部ホスト宛のため`)
        }
        throw new Error(`${result.imported}件をインポートしました（${reasons.join('、')}スキップ）`)
      }
    })
  }

  // GET の応答待ち中に確定したトグルを、後着の古い一覧で消さないための
  // 上書き。204 を受けた分だけが残り、裏付けの取れた分は loadArticles が落とす。
  const withFavoriteOverrides = (a: Article): Article =>
    a.id in favoriteOverrides ? { ...a, is_favorite: favoriteOverrides[a.id] } : a

  const effectiveArticles = articles.map(withFavoriteOverrides)
  const effectiveSelected = selectedArticle
    ? withFavoriteOverrides(selectedArticle)
    : null

  const visibleArticles = effectiveArticles.filter(
    (a) =>
      (!showFavoritesOnly || a.is_favorite) && (!showUnreadOnly || !readIds.has(a.id)),
  )

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
            showFavoritesOnly={showFavoritesOnly}
            selectedArticle={effectiveSelected}
            onToggleUnread={() => {
              setShowUnreadOnly((v) => !v)
              setSelectedArticle(null)
            }}
            onToggleFavorites={() => {
              setShowFavoritesOnly((v) => !v)
              setSelectedArticle(null)
            }}
            onSelectArticle={handleSelectArticle}
            onMarkAllRead={handleMarkAllRead}
            onShowSidebar={() => setMobileView('sidebar')}
            onToggleFavorite={handleToggleFavorite}
          />
        </section>

        <section className={`article-detail${mobileView !== 'detail' ? ' article-detail--hidden' : ''}`}>
          {effectiveSelected ? (
            <ArticleDetail
              article={effectiveSelected}
              onBack={() => setMobileView('list')}
              onToggleFavorite={() => handleToggleFavorite(effectiveSelected)}
            />
          ) : (
            <div className="detail-placeholder">記事を選択してください</div>
          )}
        </section>
      </div>
    </div>
  )
}
