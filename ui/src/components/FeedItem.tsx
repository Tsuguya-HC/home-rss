import { useState } from 'react'
import { Feed } from '../types'

interface FeedItemProps {
  feed: Feed
  unreadCount: number
  isSelected: boolean
  onSelect: () => void
  onDelete: () => Promise<void>
}

function fetchFailingSinceText(since: number | null): string {
  if (since == null) return ''
  return new Date(since * 1000).toLocaleString()
}

function failureTitle(feed: Feed): string {
  const since = fetchFailingSinceText(feed.fetch_failing_since)
  if (since === '') return feed.last_fetch_error ?? ''
  return `${feed.last_fetch_error} (${since})`
}

export function FeedItem({ feed, unreadCount, isSelected, onSelect, onDelete }: FeedItemProps) {
  const [confirmDelete, setConfirmDelete] = useState(false)

  const handleDelete = async (e: React.MouseEvent) => {
    e.stopPropagation()
    if (!confirmDelete) {
      setConfirmDelete(true)
      setTimeout(() => setConfirmDelete(false), 3000)
      return
    }
    await onDelete()
  }

  return (
    <div className={`feed-item${isSelected ? ' feed-item--active' : ''}`} onClick={onSelect}>
      <span className="feed-name">
        {feed.title || feed.url}
        {feed.last_fetch_error != null && (
          <span className="fetch-warning" title={failureTitle(feed)}>
            ⚠
          </span>
        )}
      </span>
      <span className="feed-actions">
        {unreadCount > 0 && <span className="unread-badge">{unreadCount}</span>}
        <button
          className={`delete-btn${confirmDelete ? ' delete-btn--confirm' : ''}`}
          onClick={handleDelete}
          title={confirmDelete ? '確認: もう一度クリックで削除' : '削除'}
        >
          {confirmDelete ? '!' : '×'}
        </button>
      </span>
    </div>
  )
}
