import { describe, expect, it, vi } from 'vitest'
import { fireEvent, render, screen, within } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import { Sidebar } from './Sidebar'
import { Feed } from '../types'

const feedA: Feed = {
  id: 'f1',
  url: 'https://example.com/a.xml',
  title: 'Feed A',
  site_url: null,
  etag: null,
  last_modified: null,
  last_fetched_at: null,
  created_at: null,
}

const feedB: Feed = {
  id: 'f2',
  url: 'https://example.com/b.xml',
  title: null,
  site_url: null,
  etag: null,
  last_modified: null,
  last_fetched_at: null,
  created_at: null,
}

interface SidebarCallbacks {
  onSelectFeed: (id: string | null) => void
  onAddFeed: (url: string) => Promise<void>
  onDeleteFeed: (id: string) => Promise<void>
  onImportOpml: (file: File) => Promise<void>
}

// このテストが捕まえる変異: 「すべて」ボタンの表示・選択状態・バッジ表示の欠落。
// fixture が FeedItem の props 境界（未読数キー無し・title null）を跨ぐ理由は、
// Sidebar が unreadCounts[feed.id] || 0 と title || url のフォールバックを持つため。
function renderSidebar(
  props: Partial<{
    feeds: Feed[]
    unreadCounts: Record<string, number>
    totalUnread: number
    selectedFeedId: string | null
  }> = {},
  callbacks: Partial<SidebarCallbacks> = {},
) {
  const cb: SidebarCallbacks = {
    onSelectFeed: vi.fn(),
    onAddFeed: vi.fn().mockResolvedValue(undefined),
    onDeleteFeed: vi.fn().mockResolvedValue(undefined),
    onImportOpml: vi.fn().mockResolvedValue(undefined),
  }
  Object.assign(cb, callbacks)
  render(
    <Sidebar
      feeds={props.feeds ?? [feedA, feedB]}
      unreadCounts={props.unreadCounts ?? {}}
      totalUnread={props.totalUnread ?? 0}
      selectedFeedId={props.selectedFeedId ?? null}
      onSelectFeed={cb.onSelectFeed}
      onAddFeed={cb.onAddFeed}
      onDeleteFeed={cb.onDeleteFeed}
      onImportOpml={cb.onImportOpml}
    />,
  )
  return cb
}

describe('Sidebar', () => {
  it('renders the app title', () => {
    renderSidebar({ feeds: [] })
    expect(screen.getByText('home-rss')).toBeTruthy()
  })

  it('marks すべて active only when no feed is selected', () => {
    const { rerender } = render(
      <Sidebar
        feeds={[]}
        unreadCounts={{}}
        totalUnread={0}
        selectedFeedId={null}
        onSelectFeed={vi.fn()}
        onAddFeed={vi.fn().mockResolvedValue(undefined)}
        onDeleteFeed={vi.fn().mockResolvedValue(undefined)}
        onImportOpml={vi.fn().mockResolvedValue(undefined)}
      />,
    )
    expect(screen.getByText('すべて').closest('button')?.className).toContain('feed-item--active')

    const onSelectFeed = vi.fn()
    rerender(
      <Sidebar
        feeds={[]}
        unreadCounts={{}}
        totalUnread={0}
        selectedFeedId="f1"
        onSelectFeed={onSelectFeed}
        onAddFeed={vi.fn().mockResolvedValue(undefined)}
        onDeleteFeed={vi.fn().mockResolvedValue(undefined)}
        onImportOpml={vi.fn().mockResolvedValue(undefined)}
      />,
    )
    expect(screen.getByText('すべて').closest('button')?.className).not.toContain('feed-item--active')
  })

  it('hides the total badge at zero and shows it when positive', () => {
    const { unmount } = render(
      <Sidebar
        feeds={[]}
        unreadCounts={{}}
        totalUnread={0}
        selectedFeedId={null}
        onSelectFeed={vi.fn()}
        onAddFeed={vi.fn().mockResolvedValue(undefined)}
        onDeleteFeed={vi.fn().mockResolvedValue(undefined)}
        onImportOpml={vi.fn().mockResolvedValue(undefined)}
      />,
    )
    expect(screen.getByText('すべて').closest('button')?.querySelector('.unread-badge')).toBeNull()
    unmount()

    renderSidebar({ feeds: [], totalUnread: 7 })
    expect(screen.getByText('すべて').closest('button')?.querySelector('.unread-badge')?.textContent).toBe('7')
  })

  it('renders each feed and falls back to url when title is missing', () => {
    renderSidebar()
    expect(screen.getByText('Feed A')).toBeTruthy()
    expect(screen.getByText('https://example.com/b.xml')).toBeTruthy()
  })

  it('selects a feed through its FeedItem', async () => {
    const user = userEvent.setup()
    const { onSelectFeed } = renderSidebar()
    await user.click(screen.getByText('Feed A'))
    expect(onSelectFeed).toHaveBeenCalledTimes(1)
    expect(onSelectFeed).toHaveBeenCalledWith('f1')
  })

  it('selects すべて on click', async () => {
    const user = userEvent.setup()
    const { onSelectFeed } = renderSidebar({ selectedFeedId: 'f1' })
    await user.click(screen.getByText('すべて'))
    expect(onSelectFeed).toHaveBeenCalledWith(null)
  })

  it('treats a missing unreadCounts key as zero (no badge on that feed)', () => {
    const { onSelectFeed } = renderSidebar({ unreadCounts: {} })
    expect(onSelectFeed).toBeDefined()
    const item = screen.getByText('Feed A').closest('.feed-item')
    expect(item?.querySelector('.unread-badge')).toBeNull()
  })

  it('shows per-feed unread badges when counts exist', () => {
    renderSidebar({ unreadCounts: { f1: 3 } })
    const item = screen.getByText('Feed A').closest('.feed-item')
    expect(item?.querySelector('.unread-badge')?.textContent).toBe('3')
  })

  it('deletes a feed after the FeedItem two-click confirmation', async () => {
    const user = userEvent.setup()
    const { onDeleteFeed } = renderSidebar()
    const item = screen.getByText('Feed A').closest('.feed-item')
    const deleteBtn = within(item as HTMLElement).getByTitle('削除')
    await user.click(deleteBtn)
    expect(onDeleteFeed).not.toHaveBeenCalled()
    await user.click(within(item as HTMLElement).getByTitle('確認: もう一度クリックで削除'))
    expect(onDeleteFeed).toHaveBeenCalledTimes(1)
    expect(onDeleteFeed).toHaveBeenCalledWith('f1')
  })

  it('adds a feed through the modal and closes it', async () => {
    const user = userEvent.setup()
    const { onAddFeed } = renderSidebar()
    await user.click(screen.getByText('+ フィード追加'))
    const input = screen.getByPlaceholderText('https://example.com/feed.xml')
    await user.type(input, 'https://example.com/new.xml')
    await user.click(screen.getByText('追加'))
    expect(onAddFeed).toHaveBeenCalledTimes(1)
    expect(onAddFeed).toHaveBeenCalledWith('https://example.com/new.xml')
    expect(screen.queryByPlaceholderText('https://example.com/feed.xml')).toBeNull()
  })

  it('imports an OPML file through the hidden file input', async () => {
    const { onImportOpml } = renderSidebar()
    const file = new File(['<opml></opml>'], 'feeds.opml', { type: 'text/xml' })
    const input = document.querySelector('input[type="file"]') as HTMLInputElement
    fireEvent.change(input, { target: { files: [file] } })
    expect(onImportOpml).toHaveBeenCalledTimes(1)
    expect(onImportOpml).toHaveBeenCalledWith(file)
  })

  it('ignores an OPML change with no file selected', async () => {
    const { onImportOpml } = renderSidebar()
    const input = document.querySelector('input[type="file"]') as HTMLInputElement
    fireEvent.change(input, { target: { files: [] } })
    expect(onImportOpml).not.toHaveBeenCalled()
  })
})
