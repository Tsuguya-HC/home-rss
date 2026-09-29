import { describe, expect, it, vi } from 'vitest'
import { render, screen } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import { ArticleListPanel } from './ArticleListPanel'
import { Article } from '../types'

function makeArticle(id: string, title: string): Article {
  return {
    id,
    feed_id: 'f1',
    url: `https://example.com/${id}`,
    title,
    content: null,
    author: null,
    published_at: null,
    fetched_at: null,
    image_url: null,
  }
}

const first = makeArticle('a1', 'First article')
const second = makeArticle('a2', 'Second article')

interface Options {
  articles?: Article[]
  loading?: boolean
  showUnreadOnly?: boolean
  selectedArticle?: Article | null
}

function renderPanel({
  articles = [first, second],
  loading = false,
  showUnreadOnly = false,
  selectedArticle = null,
}: Options = {}) {
  const handlers = {
    onToggleUnread: vi.fn<() => void>(),
    onSelectArticle: vi.fn<(article: Article) => void>(),
    onMarkAllRead: vi.fn<() => void>(),
    onShowSidebar: vi.fn<() => void>(),
  }
  const view = render(
    <ArticleListPanel
      articles={articles}
      loading={loading}
      showUnreadOnly={showUnreadOnly}
      selectedArticle={selectedArticle}
      {...handlers}
    />,
  )
  return { ...handlers, ...view }
}

describe('ArticleListPanel', () => {
  it('lists every article in order', () => {
    const { container } = renderPanel()

    const titles = [...container.querySelectorAll('.article-item-title')].map(
      (el) => el.textContent,
    )
    expect(titles).toEqual(['First article', 'Second article'])
    expect(screen.queryByText('読み込み中...')).toBeNull()
    expect(screen.queryByText('記事がありません')).toBeNull()
  })

  it('shows the empty message when there are no articles', () => {
    const { container } = renderPanel({ articles: [] })

    expect(screen.getByText('記事がありません')).toBeTruthy()
    expect(container.querySelector('.article-list-items')).toBeNull()
    expect(screen.queryByText('読み込み中...')).toBeNull()
  })

  it('shows the loading message instead of an empty list while loading', () => {
    renderPanel({ articles: [], loading: true })

    expect(screen.getByText('読み込み中...')).toBeTruthy()
    expect(screen.queryByText('記事がありません')).toBeNull()
  })

  it('shows the loading message instead of the previous articles while loading', () => {
    renderPanel({ loading: true })

    expect(screen.getByText('読み込み中...')).toBeTruthy()
    expect(screen.queryByText('First article')).toBeNull()
  })

  it('marks only the selected article as active', () => {
    const { container } = renderPanel({ selectedArticle: second })

    const active = container.querySelectorAll('.article-item--active')
    expect(active).toHaveLength(1)
    expect(active[0].textContent).toContain('Second article')
  })

  it('marks no article as active when none is selected', () => {
    const { container } = renderPanel({ selectedArticle: null })

    expect(container.querySelector('.article-item--active')).toBeNull()
  })

  it('calls onSelectArticle with the clicked article', async () => {
    const user = userEvent.setup()
    const { onSelectArticle } = renderPanel()

    await user.click(screen.getByText('Second article'))

    expect(onSelectArticle).toHaveBeenCalledTimes(1)
    expect(onSelectArticle).toHaveBeenCalledWith(second)
  })

  it('reflects showUnreadOnly in the checkbox', () => {
    const on = renderPanel({ showUnreadOnly: true })
    expect((screen.getByLabelText('未読のみ') as HTMLInputElement).checked).toBe(true)
    on.unmount()

    renderPanel({ showUnreadOnly: false })
    expect((screen.getByLabelText('未読のみ') as HTMLInputElement).checked).toBe(false)
  })

  it('calls onToggleUnread when the checkbox is clicked', async () => {
    const user = userEvent.setup()
    const { onToggleUnread } = renderPanel()

    await user.click(screen.getByLabelText('未読のみ'))

    expect(onToggleUnread).toHaveBeenCalledTimes(1)
  })

  it('calls onMarkAllRead when 全既読 is clicked', async () => {
    const user = userEvent.setup()
    const { onMarkAllRead, onShowSidebar } = renderPanel()

    await user.click(screen.getByText('全既読'))

    expect(onMarkAllRead).toHaveBeenCalledTimes(1)
    expect(onShowSidebar).not.toHaveBeenCalled()
  })

  it('calls onShowSidebar when the menu button is clicked', async () => {
    const user = userEvent.setup()
    const { onShowSidebar, onMarkAllRead } = renderPanel()

    await user.click(screen.getByTitle('メニュー'))

    expect(onShowSidebar).toHaveBeenCalledTimes(1)
    expect(onMarkAllRead).not.toHaveBeenCalled()
  })

  it('keeps the toolbar usable while loading and when empty', async () => {
    const user = userEvent.setup()
    const { onMarkAllRead } = renderPanel({ articles: [], loading: true })

    await user.click(screen.getByText('全既読'))

    expect(onMarkAllRead).toHaveBeenCalledTimes(1)
  })
})
