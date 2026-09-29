import { describe, expect, it, vi } from 'vitest'
import { fireEvent, render, screen } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import { ArticleListPanel } from './ArticleListPanel'
import { Article } from '../types'

function article(overrides: Partial<Article> = {}): Article {
  return {
    id: 'a1',
    feed_id: 'f1',
    url: 'https://example.com/a1',
    title: 'Article 1',
    content: null,
    author: null,
    published_at: null,
    fetched_at: null,
    image_url: null,
    ...overrides,
  }
}

const articleA = article({ id: 'a1', title: 'Article 1' })
const articleB = article({ id: 'a2', title: 'Article 2', url: 'https://example.com/a2' })

interface PanelCallbacks {
  onToggleUnread: () => void
  onSelectArticle: (a: Article) => void
  onMarkAllRead: () => void
  onShowSidebar: () => void
}

// このテストが捕まえる変異: loading/空/通常の 3 分岐の取り違えとコールバックの付け間違い。
function renderPanel(
  props: Partial<{
    articles: Article[]
    loading: boolean
    showUnreadOnly: boolean
    selectedArticle: Article | null
  }> = {},
  callbacks: Partial<PanelCallbacks> = {},
) {
  const cb: PanelCallbacks = {
    onToggleUnread: vi.fn(),
    onSelectArticle: vi.fn(),
    onMarkAllRead: vi.fn(),
    onShowSidebar: vi.fn(),
  }
  Object.assign(cb, callbacks)
  render(
    <ArticleListPanel
      articles={props.articles ?? []}
      loading={props.loading ?? false}
      showUnreadOnly={props.showUnreadOnly ?? false}
      selectedArticle={props.selectedArticle ?? null}
      onToggleUnread={cb.onToggleUnread}
      onSelectArticle={cb.onSelectArticle}
      onMarkAllRead={cb.onMarkAllRead}
      onShowSidebar={cb.onShowSidebar}
    />,
  )
  return cb
}

describe('ArticleListPanel', () => {
  it('shows loading instead of articles while loading', () => {
    renderPanel({ articles: [articleA], loading: true })
    expect(screen.getByText('読み込み中...')).toBeTruthy()
    expect(screen.queryByText('Article 1')).toBeNull()
  })

  it('shows the empty message when not loading and there are no articles', () => {
    renderPanel({ articles: [], loading: false })
    expect(screen.getByText('記事がありません')).toBeTruthy()
  })

  it('renders one row per article', () => {
    renderPanel({ articles: [articleA, articleB] })
    expect(screen.getByText('Article 1')).toBeTruthy()
    expect(screen.getByText('Article 2')).toBeTruthy()
  })

  it('reflects showUnreadOnly in the checkbox', () => {
    const { unmount } = render(
      <ArticleListPanel
        articles={[]}
        loading={false}
        showUnreadOnly={false}
        selectedArticle={null}
        onToggleUnread={vi.fn()}
        onSelectArticle={vi.fn()}
        onMarkAllRead={vi.fn()}
        onShowSidebar={vi.fn()}
      />,
    )
    expect((screen.getByLabelText('未読のみ') as HTMLInputElement).checked).toBe(false)
    unmount()

    renderPanel({ showUnreadOnly: true })
    expect((screen.getByLabelText('未読のみ') as HTMLInputElement).checked).toBe(true)
  })

  it('toggles unread-only through the checkbox', () => {
    const { onToggleUnread } = renderPanel()
    fireEvent.click(screen.getByLabelText('未読のみ'))
    expect(onToggleUnread).toHaveBeenCalledTimes(1)
  })

  it('marks all read through the button', async () => {
    const user = userEvent.setup()
    const { onMarkAllRead } = renderPanel({ articles: [articleA] })
    await user.click(screen.getByText('全既読'))
    expect(onMarkAllRead).toHaveBeenCalledTimes(1)
  })

  it('shows the sidebar through the hamburger button', async () => {
    const user = userEvent.setup()
    const { onShowSidebar } = renderPanel()
    await user.click(screen.getByTitle('メニュー'))
    expect(onShowSidebar).toHaveBeenCalledTimes(1)
  })

  it('selects the clicked article', async () => {
    const user = userEvent.setup()
    const { onSelectArticle } = renderPanel({ articles: [articleA, articleB] })
    await user.click(screen.getByText('Article 2'))
    expect(onSelectArticle).toHaveBeenCalledTimes(1)
    expect(onSelectArticle).toHaveBeenCalledWith(articleB)
  })

  it('marks only the selected article active', () => {
    renderPanel({ articles: [articleA, articleB], selectedArticle: articleB })
    const active = document.querySelectorAll('.article-item--active')
    expect(active.length).toBe(1)
    expect(active[0].textContent).toContain('Article 2')
  })

  it('marks nothing active when no article is selected', () => {
    renderPanel({ articles: [articleA], selectedArticle: null })
    expect(document.querySelector('.article-item--active')).toBeNull()
  })
})
