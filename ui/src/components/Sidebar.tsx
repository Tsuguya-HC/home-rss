import { useRef, useState } from 'react'
import { Feed } from '../types'
import { FeedItem } from './FeedItem'
import { AddFeedModal } from './AddFeedModal'

interface SidebarProps {
  feeds: Feed[]
  unreadCounts: Record<string, number>
  totalUnread: number
  selectedFeedId: string | null
  onSelectFeed: (id: string | null) => void
  onAddFeed: (url: string) => Promise<void>
  onDeleteFeed: (id: string) => Promise<void>
  onImportOpml: (file: File) => Promise<void>
}

export function Sidebar({
  feeds,
  unreadCounts,
  totalUnread,
  selectedFeedId,
  onSelectFeed,
  onAddFeed,
  onDeleteFeed,
  onImportOpml,
}: SidebarProps) {
  const [showAddFeed, setShowAddFeed] = useState(false)
  const fileInputRef = useRef<HTMLInputElement>(null)

  const handleOpmlChange = async (e: React.ChangeEvent<HTMLInputElement>) => {
    const file = e.target.files?.[0]
    if (!file) return
    await onImportOpml(file)
    e.target.value = ''
  }

  return (
    <div className="sidebar-inner">
      <div className="sidebar-header">
        <h1 className="app-title">home-rss</h1>
      </div>

      <nav className="feed-list">
        <button
          className={`feed-item${selectedFeedId === null ? ' feed-item--active' : ''}`}
          onClick={() => onSelectFeed(null)}
        >
          <span className="feed-name">すべて</span>
          {totalUnread > 0 && (
            <span className="unread-badge">{totalUnread}</span>
          )}
        </button>

        {feeds.map((feed) => (
          <FeedItem
            key={feed.id}
            feed={feed}
            unreadCount={unreadCounts[feed.id] || 0}
            isSelected={selectedFeedId === feed.id}
            onSelect={() => onSelectFeed(feed.id)}
            onDelete={() => onDeleteFeed(feed.id)}
          />
        ))}
      </nav>

      <div className="sidebar-actions">
        <button className="btn btn--primary btn--full" onClick={() => setShowAddFeed(true)}>
          + フィード追加
        </button>
        <button
          className="btn btn--secondary btn--full"
          onClick={() => fileInputRef.current?.click()}
        >
          OPML インポート
        </button>
        <input
          ref={fileInputRef}
          type="file"
          accept=".opml,.xml"
          style={{ display: 'none' }}
          onChange={handleOpmlChange}
        />
      </div>

      {showAddFeed && (
        <AddFeedModal
          onAdd={async (url) => {
            await onAddFeed(url)
            setShowAddFeed(false)
          }}
          onClose={() => setShowAddFeed(false)}
        />
      )}
    </div>
  )
}
