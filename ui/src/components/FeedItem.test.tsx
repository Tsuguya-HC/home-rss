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
  it('shows a warning mark with the reason and failing-since time for a failing feed', () => {
    // 失敗中のフィードだけ、削除確認の `!` とは別物と分かる警告マーク (⚠) を出す (#245)。
    const failing: Feed = {
      ...feed,
      last_fetch_error: 'HTTP 404',
      fetch_failing_since: 1_757_894_400,
    }
    render(
      <FeedItem
        feed={failing}
        unreadCount={0}
        isSelected={false}
        onSelect={() => {}}
        onDelete={async () => {}}
      />,
    )

    const mark = screen.getByText('⚠')
    expect(mark.getAttribute('title')).toContain('HTTP 404')
    expect(mark.getAttribute('title')).toContain('2025')
  })

  it('shows no warning mark for a feed without a failure record', () => {
    renderItem()

    expect(screen.queryByText('⚠')).toBeNull()
  })

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
})
