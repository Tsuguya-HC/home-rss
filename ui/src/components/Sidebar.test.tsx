import { describe, expect, it, vi } from 'vitest'
import { act, fireEvent, render, screen } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import { Sidebar } from './Sidebar'
import { Feed } from '../types'

function makeFeed(id: string, title: string | null): Feed {
  return {
    id,
    url: `https://example.com/${id}.xml`,
    title,
    site_url: null,
    etag: null,
    last_modified: null,
    last_fetched_at: null,
    created_at: null,
    last_fetch_error: null,
    fetch_failing_since: null,
  }
}

const alpha = makeFeed('f1', 'Alpha')
const beta = makeFeed('f2', 'Beta')

interface Options {
  feeds?: Feed[]
  unreadCounts?: Record<string, number>
  totalUnread?: number
  selectedFeedId?: string | null
}

function renderSidebar({
  feeds = [alpha, beta],
  unreadCounts = {},
  totalUnread = 0,
  selectedFeedId = null,
}: Options = {}) {
  const handlers = {
    onSelectFeed: vi.fn<(id: string | null) => void>(),
    onAddFeed: vi.fn<(url: string) => Promise<void>>().mockResolvedValue(undefined),
    onDeleteFeed: vi.fn<(id: string) => Promise<void>>().mockResolvedValue(undefined),
    onImportOpml: vi.fn<(file: File) => Promise<void>>().mockResolvedValue(undefined),
  }
  const view = render(
    <Sidebar
      feeds={feeds}
      unreadCounts={unreadCounts}
      totalUnread={totalUnread}
      selectedFeedId={selectedFeedId}
      {...handlers}
    />,
  )
  return { ...handlers, ...view }
}

const fileInput = (container: HTMLElement) =>
  container.querySelector<HTMLInputElement>('input[type="file"]')!

describe('Sidebar', () => {
  it('renders the すべて entry and one entry per feed, falling back to the url without a title', () => {
    renderSidebar({ feeds: [alpha, makeFeed('f3', null)] })

    expect(screen.getByText('すべて')).toBeTruthy()
    expect(screen.getByText('Alpha')).toBeTruthy()
    expect(screen.getByText('https://example.com/f3.xml')).toBeTruthy()
  })

  it('renders only the すべて entry when there are no feeds', () => {
    const { container } = renderSidebar({ feeds: [] })

    expect(screen.getByText('すべて')).toBeTruthy()
    expect(container.querySelectorAll('.feed-item')).toHaveLength(1)
  })

  it('shows the total unread badge only when it is above zero', () => {
    const unread = renderSidebar({ totalUnread: 7 })
    expect(unread.container.querySelector('.unread-badge')!.textContent).toBe('7')
    unread.unmount()

    const none = renderSidebar({ totalUnread: 0 })
    expect(none.container.querySelector('.unread-badge')).toBeNull()
  })

  it('shows each feed its own unread count, and no badge for a feed without one', () => {
    const { container } = renderSidebar({ unreadCounts: { f1: 3, f2: 0 } })

    const badges = [...container.querySelectorAll('.unread-badge')]
    expect(badges.map((b) => b.textContent)).toEqual(['3'])
    expect(
      screen.getByText('Alpha').parentElement!.querySelector('.unread-badge')!.textContent,
    ).toBe('3')
  })

  it('does not fall back to the total for a feed without a count', () => {
    const { container } = renderSidebar({ totalUnread: 7, unreadCounts: {} })

    expect([...container.querySelectorAll('.unread-badge')].map((b) => b.textContent)).toEqual([
      '7',
    ])
  })

  it('ignores unread counts for feeds that are not listed', () => {
    const { container } = renderSidebar({ unreadCounts: { gone: 5 } })

    expect(container.querySelector('.unread-badge')).toBeNull()
  })

  it('marks すべて as active when no feed is selected', () => {
    const { container } = renderSidebar({ selectedFeedId: null })

    const active = container.querySelectorAll('.feed-item--active')
    expect(active).toHaveLength(1)
    expect(active[0].textContent).toContain('すべて')
  })

  it('marks only the selected feed as active', () => {
    const { container } = renderSidebar({ selectedFeedId: 'f2' })

    const active = container.querySelectorAll('.feed-item--active')
    expect(active).toHaveLength(1)
    expect(active[0].textContent).toContain('Beta')
  })

  it('calls onSelectFeed with null for すべて and with the id for a feed', async () => {
    const user = userEvent.setup()
    const { onSelectFeed } = renderSidebar({ selectedFeedId: 'f1' })

    await user.click(screen.getByText('すべて'))
    expect(onSelectFeed).toHaveBeenLastCalledWith(null)

    await user.click(screen.getByText('Beta'))
    expect(onSelectFeed).toHaveBeenLastCalledWith('f2')
    expect(onSelectFeed).toHaveBeenCalledTimes(2)
  })

  it('calls onDeleteFeed with the id of the feed whose delete was confirmed', async () => {
    const user = userEvent.setup()
    const { onDeleteFeed, onSelectFeed } = renderSidebar()

    const betaRow = screen.getByText('Beta').closest('.feed-item') as HTMLElement
    const deleteButton = betaRow.querySelector('.delete-btn') as HTMLElement
    await user.click(deleteButton)
    await user.click(deleteButton)

    expect(onDeleteFeed).toHaveBeenCalledTimes(1)
    expect(onDeleteFeed).toHaveBeenCalledWith('f2')
    expect(onSelectFeed).not.toHaveBeenCalled()
  })

  it('opens the add-feed modal on demand and not before', async () => {
    const user = userEvent.setup()
    renderSidebar()
    expect(screen.queryByPlaceholderText('https://example.com/feed.xml')).toBeNull()

    await user.click(screen.getByText('+ フィード追加'))

    expect(screen.getByPlaceholderText('https://example.com/feed.xml')).toBeTruthy()
  })

  it('closes the add-feed modal on cancel without adding', async () => {
    const user = userEvent.setup()
    const { onAddFeed } = renderSidebar()
    await user.click(screen.getByText('+ フィード追加'))

    await user.click(screen.getByText('キャンセル'))

    expect(screen.queryByPlaceholderText('https://example.com/feed.xml')).toBeNull()
    expect(onAddFeed).not.toHaveBeenCalled()
  })

  it('adds the entered url and closes the modal once onAddFeed resolves', async () => {
    const user = userEvent.setup()
    const { onAddFeed } = renderSidebar()
    await user.click(screen.getByText('+ フィード追加'))

    await user.type(
      screen.getByPlaceholderText('https://example.com/feed.xml'),
      'https://example.com/new.xml',
    )
    await user.click(screen.getByText('追加'))

    expect(onAddFeed).toHaveBeenCalledTimes(1)
    expect(onAddFeed).toHaveBeenCalledWith('https://example.com/new.xml')
    expect(screen.queryByPlaceholderText('https://example.com/feed.xml')).toBeNull()
  })

  it('keeps the add-feed modal open while onAddFeed is pending', async () => {
    const user = userEvent.setup()
    let resolveAdd!: () => void
    const { onAddFeed } = renderSidebar()
    onAddFeed.mockReturnValue(new Promise<void>((resolve) => (resolveAdd = resolve)))
    await user.click(screen.getByText('+ フィード追加'))
    await user.type(
      screen.getByPlaceholderText('https://example.com/feed.xml'),
      'https://example.com/new.xml',
    )

    await user.click(screen.getByText('追加'))

    expect(screen.getByPlaceholderText('https://example.com/feed.xml')).toBeTruthy()
    await act(async () => {
      resolveAdd()
    })
    expect(screen.queryByPlaceholderText('https://example.com/feed.xml')).toBeNull()
  })

  it('opens the file picker when OPML インポート is clicked', async () => {
    const user = userEvent.setup()
    const { container } = renderSidebar()
    const onPickerClick = vi.fn<() => void>()
    fileInput(container).addEventListener('click', onPickerClick)

    await user.click(screen.getByText('OPML インポート'))

    expect(onPickerClick).toHaveBeenCalledTimes(1)
  })

  it('only accepts opml and xml files in the picker', () => {
    const { container } = renderSidebar()

    expect(fileInput(container).getAttribute('accept')).toBe('.opml,.xml')
  })

  it('hands the chosen file to onImportOpml', async () => {
    const user = userEvent.setup()
    const { container, onImportOpml } = renderSidebar()
    const file = new File(['<opml/>'], 'feeds.opml', { type: 'text/x-opml' })

    await user.upload(fileInput(container), file)

    expect(onImportOpml).toHaveBeenCalledTimes(1)
    expect(onImportOpml.mock.calls[0][0]).toBe(file)
  })

  it('clears the picker after an import so the same file can be chosen again', async () => {
    const user = userEvent.setup()
    const { container, onImportOpml } = renderSidebar()
    const input = fileInput(container)
    const file = new File(['<opml/>'], 'feeds.opml')

    await user.upload(input, file)
    expect(input.value).toBe('')
    await user.upload(input, file)

    expect(onImportOpml).toHaveBeenCalledTimes(2)
  })

  it('does not call onImportOpml when the picker changes to no file', () => {
    const { container, onImportOpml } = renderSidebar()

    fireEvent.change(fileInput(container), { target: { files: [] } })

    expect(onImportOpml).not.toHaveBeenCalled()
  })
})
