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
}

function renderItem(onDelete = vi.fn().mockResolvedValue(undefined), onSelect = vi.fn()) {
  render(
    <FeedItem feed={feed} unreadCount={0} isSelected={false} onSelect={onSelect} onDelete={onDelete} />,
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
})
