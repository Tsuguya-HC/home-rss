import { useState, useEffect, useCallback, useRef } from 'react'
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
  const [favoriteIds, setFavoriteIds] = useState<Set<string>>(new Set())
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

  // クリック時点の実表示を同刻に読むための ref。再取得がサーバーの
  // 実データで作り直すので初期値は null のまま (#152)。
  const favoriteToggleState = useRef<Set<string> | null>(null)
  // 同じ記事へのトグル要求をクリック順に完了させる鎖の末尾 (#152)。
  // 並列に投げると完了順がクリック順と入れ替わり、表示とサーバーの
  // 実データが食い違ったまま固まる。鎖を抜ける経路は無いので
  // 直列化が崩れることはない。
  const favoriteToggleChains = useRef(new Map<string, Promise<void>>())
  // 進行中の楽観更新を識別する記事ごとの世代 (#152)。新しいクリックの
  // 楽観表示を古い失敗が巻き戻すことがないよう、末尾の世代だけが
  // 画面を確定させる。
  const favoriteToggleGenerations = useRef(new Map<string, number>())

  const loadArticles = useCallback(async () => {
    setLoadingArticles(true)
    try {
      const articles = await api.getArticles(selectedFeedId, showUnreadOnly, showFavoritesOnly)
      setArticles(articles)
      setReadIds(new Set())
      // 再取得はサーバーの実データを表示の正とするので
      // 楽観中の ref も世代も捨てる (#152)。
      favoriteToggleState.current = new Set(
        articles.filter((a) => a.is_favorite).map((a) => a.id),
      )
      favoriteToggleGenerations.current.clear()
      setFavoriteIds(new Set(favoriteToggleState.current))
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
      // 連打では間に挟まる再レンダーが保証されないので、state ではなく
      // 同刻に進む ref をそのクリック時点の実表示として読む (#152)。
      const shownIds = favoriteToggleState.current ?? favoriteIds
      const isFavorite = shownIds.has(article.id)
      const generation = (favoriteToggleGenerations.current.get(article.id) ?? 0) + 1
      favoriteToggleGenerations.current.set(article.id, generation)
      const optimistic = new Set(shownIds)
      if (isFavorite) {
        optimistic.delete(article.id)
      } else {
        optimistic.add(article.id)
      }
      favoriteToggleState.current = optimistic
      setFavoriteIds(new Set(optimistic))
      const tail = favoriteToggleChains.current.get(article.id) ?? Promise.resolve()
      const turn = tail.then(async () => {
        try {
          if (isFavorite) {
            await api.unmarkFavorite(article.id)
          } else {
            await api.markFavorite(article.id)
          }
          setArticles((prev) =>
            prev.map((a) => (a.id === article.id ? { ...a, is_favorite: !isFavorite } : a)),
          )
          setSelectedArticle((prev) =>
            prev?.id === article.id ? { ...prev, is_favorite: !isFavorite } : prev,
          )
          if (showFavoritesOnly && isFavorite) {
            setArticles((prev) => prev.filter((a) => a.id !== article.id))
          }
        } catch (e) {
          // 直列化で最新だけが残るので、巻き戻しは自分の世代が
          // 末尾のときだけ行う (#152)。
          if (favoriteToggleGenerations.current.get(article.id) === generation) {
            const rolledBack = new Set(favoriteToggleState.current ?? favoriteIds)
            if (isFavorite) {
              rolledBack.add(article.id)
            } else {
              rolledBack.delete(article.id)
            }
            favoriteToggleState.current = rolledBack
            setFavoriteIds(new Set(rolledBack))
          }
          throw e
        }
      })
      // 失敗しても鎖だけは保つよう、末尾には解決だけ進める (#152)。
      favoriteToggleChains.current.set(
        article.id,
        turn.then(
          () => undefined,
          () => undefined,
        ),
      )
      await turn
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
            showFavoritesOnly={showFavoritesOnly}
            favoriteIds={favoriteIds}
            selectedArticle={selectedArticle}
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
            onToggleFavorite={handleToggleFavorite}
            onShowSidebar={() => setMobileView('sidebar')}
          />
        </section>

        <section className={`article-detail${mobileView !== 'detail' ? ' article-detail--hidden' : ''}`}>
          {selectedArticle ? (
            <ArticleDetail
              article={selectedArticle}
              isFavorite={favoriteIds.has(selectedArticle.id)}
              onToggleFavorite={() => handleToggleFavorite(selectedArticle)}
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
