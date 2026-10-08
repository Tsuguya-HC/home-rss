import { describe, expect, it, vi } from 'vitest'
import { act, fireEvent, render, screen } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import { FeedItem } from './FeedItem'
import { Feed } from '../types'

const feed: Feed = {
  id: 'f1',
  url: 'https://example.com/feed',
  title: 'Example',
  site_url: null,
  etag: null,
  last_modified: null,
  last_fetched_at: null,
  created_at: null,
  last_fetch_error: null,
  fetch_failing_since: null,
}

function renderItem(
  onDelete = vi.fn<() => Promise<void>>().mockResolvedValue(undefined),
  onSelect = vi.fn<() => void>(),
) {
  render(
    <FeedItem
      feed={feed}
      unreadCount={0}
      isSelected={false}
      onSelect={onSelect}
      onDelete={onDelete}
    />,
  )
  return { onDelete, onSelect }
}

describe('FeedItem', () => {
  it('deletes only on the second click', async () => {
    const user = userEvent.setup()
    const { onDelete, onSelect } = renderItem()

    await user.click(screen.getByTitle('削除'))
    expect(onDelete).not.toHaveBeenCalled()

    await user.click(screen.getByTitle('確認: もう一度クリックで削除'))
    expect(onDelete).toHaveBeenCalledTimes(1)
    expect(onSelect).not.toHaveBeenCalled()
  })

  it('drops the confirmation after three seconds', () => {
    vi.useFakeTimers()
    try {
      const { onDelete } = renderItem()

      fireEvent.click(screen.getByTitle('削除'))
      act(() => {
        vi.advanceTimersByTime(3000)
      })
      fireEvent.click(screen.getByTitle('削除'))
      expect(onDelete).not.toHaveBeenCalled()
    } finally {
      vi.useRealTimers()
    }
  })

  it('shows a failure warning with reason and start time', () => {
    // Catches a failing feed looking like a merely stale one: the warning
    // mark must carry the record's reason and when it started failing (#245).
    const failing = {
      ...feed,
      last_fetch_error: 'HTTP 404',
      fetch_failing_since: 1757894400,
    } as unknown as Feed
    render(
      <FeedItem
        feed={failing}
        unreadCount={0}
        isSelected={false}
        onSelect={() => {}}
        onDelete={async () => {}}
      />,
    )
    const mark = screen.getByTitle(/HTTP 404/)
    expect(mark.textContent).toContain('⚠')
    expect(mark.getAttribute('title')).toContain(String(new Date(1757894400 * 1000).getFullYear()))
  })

  it('shows no failure warning for a healthy feed', () => {
    renderItem()
    expect(screen.queryByText('⚠')).toBeNull()
  })

  it('shows unknown start time when the record has no timestamp', () => {
    // Catches the warning title breaking on a record without
    // fetch_failing_since: the reason must still show with a fallback (#245).
    render(
      <FeedItem
        feed={{ ...feed, last_fetch_error: 'HTTP 500', fetch_failing_since: null }}
        unreadCount={0}
        isSelected={false}
        onSelect={() => {}}
        onDelete={async () => {}}
      />,
    )
    const mark = screen.getByTitle(/HTTP 500/)
    expect(mark.textContent).toContain('⚠')
    expect(mark.getAttribute('title')).toContain('日時不明')
  })
})
