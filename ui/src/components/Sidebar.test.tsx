import { describe, expect, it, vi } from 'vitest'
import { act, fireEvent, render, screen, within } from '@testing-library/react'
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
// 未読数のフォールバック（unreadCounts[feed.id] || 0）は Sidebar が、
// title が無いときの url 表示（feed.title || feed.url）は子コンポーネント FeedItem が持つため。
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

  // このテストが捕まえる変異: AddFeedModal の onAdd 完了を待たずにモーダルを
  // 閉じる変更（await onAddFeed(url) の await 剥がし）。解決前のモーダルの
  // 「追加中...」表示（AddFeedModal が onAdd の解決まで loading を保つこと）が
  // 無くなれば、await が外れている。
  it('keeps the add-feed modal open until adding finishes', async () => {
    const user = userEvent.setup()
    let resolveAdd!: (value: undefined) => void
    const onAddFeed = vi
      .fn()
      .mockImplementation(() => new Promise<undefined>((resolve) => { resolveAdd = resolve }))
    renderSidebar({}, { onAddFeed })
    await user.click(screen.getByText('+ フィード追加'))
    await user.type(
      screen.getByPlaceholderText('https://example.com/feed.xml'),
      'https://example.com/new.xml',
    )
    await user.click(screen.getByText('追加'))
    expect(onAddFeed).toHaveBeenCalledTimes(1)
    expect(screen.getByText('追加中...')).toBeTruthy()
    expect(screen.queryByPlaceholderText('https://example.com/feed.xml')).not.toBeNull()
    await act(async () => {
      resolveAdd(undefined)
    })
    expect(screen.queryByPlaceholderText('https://example.com/feed.xml')).toBeNull()
  })

  // このテストが捕まえる変異: handleOpmlChange の e.target.value = '' の削除。
  // jsdom では fireEvent.change で files は反映されるが input.value は空のままになる
  // （2026-09-29 実測。再現: input type=file に fireEvent.change(input,
  // { target: { files: [file] } }) した直後に input.value を読むと ''）ため、
  // value の読み取りではリセットを観測できず、setter の記録で '' 代入そのものを観測する。
  // 記録は対象の input 要素にだけ置き、fireEvent.change の前に仕掛ける。
  it('resets the OPML file input after an import', async () => {
    const { onImportOpml } = renderSidebar()
    const input = document.querySelector('input[type="file"]') as HTMLInputElement
    const assignedValues: unknown[] = []
    Object.defineProperty(input, 'value', {
      configurable: true,
      get() {
        return ''
      },
      set(v: unknown) {
        assignedValues.push(v)
      },
    })
    const file = new File(['<opml></opml>'], 'feeds.opml', { type: 'text/xml' })
    fireEvent.change(input, { target: { files: [file] } })
    expect(onImportOpml).toHaveBeenCalledTimes(1)
    await act(async () => {})
    expect(assignedValues).toEqual([''])
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
