import { describe, expect, it, vi } from 'vitest'
import { fireEvent, render, screen } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import { ArticleListItem } from './ArticleListItem'
import { formatDate } from '../lib/date'
import { Article } from '../types'

const article: Article = {
  id: 'a1',
  feed_id: 'f1',
  url: 'https://example.com/a1',
  title: 'First article',
  content: null,
  author: null,
  published_at: 1735732800,
  fetched_at: null,
  image_url: 'https://example.com/a1.png',
}

function renderItem(
  overrides: Partial<Article> = {},
  isSelected = false,
  onClick = vi.fn<() => void>(),
) {
  const view = render(
    <ArticleListItem
      article={{ ...article, ...overrides }}
      isSelected={isSelected}
      onClick={onClick}
    />,
  )
  const rerenderWith = (next: Partial<Article>) =>
    view.rerender(
      <ArticleListItem
        article={{ ...article, ...next }}
        isSelected={isSelected}
        onClick={onClick}
      />,
    )
  return { onClick, rerenderWith, ...view }
}

const thumbnail = (container: HTMLElement) => container.querySelector('img')

describe('ArticleListItem', () => {
  it('renders the title and the formatted publication date', () => {
    renderItem()

    expect(screen.getByText('First article')).toBeTruthy()
    expect(screen.getByText(formatDate(1735732800))).toBeTruthy()
  })

  it('renders no date when published_at is missing', () => {
    const { container } = renderItem({ published_at: null })

    expect(screen.getByText('First article')).toBeTruthy()
    expect(container.querySelector('.article-item-date')).toBeNull()
  })

  it('renders the thumbnail with the image url and privacy attributes', () => {
    const { container } = renderItem()

    const img = thumbnail(container)!
    expect(img.getAttribute('src')).toBe('https://example.com/a1.png')
    expect(img.getAttribute('referrerpolicy')).toBe('no-referrer')
    expect(img.getAttribute('loading')).toBe('lazy')
    expect(container.querySelector('.article-item--with-thumbnail')).not.toBeNull()
  })

  it('renders no thumbnail when image_url is missing', () => {
    const { container } = renderItem({ image_url: null })

    expect(thumbnail(container)).toBeNull()
    expect(container.querySelector('.article-item--with-thumbnail')).toBeNull()
  })

  it('marks the selected item as active and only that one', () => {
    const selected = renderItem({}, true)
    expect(selected.container.querySelector('.article-item--active')).not.toBeNull()
    selected.unmount()

    const unselected = renderItem({}, false)
    expect(unselected.container.querySelector('.article-item--active')).toBeNull()
  })

  it('calls onClick when the item is clicked', async () => {
    const user = userEvent.setup()
    const { onClick } = renderItem()

    await user.click(screen.getByText('First article'))

    expect(onClick).toHaveBeenCalledTimes(1)
  })

  it('hides the thumbnail when the image fails to load', () => {
    const { container } = renderItem()

    fireEvent.error(thumbnail(container)!)

    expect(thumbnail(container)).toBeNull()
    expect(screen.getByText('First article')).toBeTruthy()
  })

  // The thumbnail layout class follows the article's image_url, not whether the image
  // is currently displayed, so the row keeps its layout after a load failure.
  it('keeps the thumbnail layout class after the image fails to load', () => {
    const { container } = renderItem()

    fireEvent.error(thumbnail(container)!)

    expect(container.querySelector('.article-item--with-thumbnail')).not.toBeNull()
  })

  // Only `error` hides the thumbnail; the component has no `load` handler, so this
  // fails if one is added that hides (or replaces) the image.
  it('does not hide the thumbnail when the image finishes loading', () => {
    const { container } = renderItem()

    fireEvent.load(thumbnail(container)!)

    expect(thumbnail(container)!.getAttribute('src')).toBe('https://example.com/a1.png')
  })

  it('shows the thumbnail again when the image url changes after a failure', () => {
    const { container, rerenderWith } = renderItem()
    fireEvent.error(thumbnail(container)!)
    expect(thumbnail(container)).toBeNull()

    rerenderWith({ image_url: 'https://example.com/a1-new.png' })

    expect(thumbnail(container)!.getAttribute('src')).toBe('https://example.com/a1-new.png')
  })

  it('shows the thumbnail again when another article reuses the same image url', () => {
    const { container, rerenderWith } = renderItem()
    fireEvent.error(thumbnail(container)!)
    expect(thumbnail(container)).toBeNull()

    rerenderWith({ id: 'a2', title: 'Second article' })

    expect(screen.getByText('Second article')).toBeTruthy()
    expect(thumbnail(container)!.getAttribute('src')).toBe('https://example.com/a1.png')
  })

  it('keeps the thumbnail hidden when unrelated fields of the same article change', () => {
    const { container, rerenderWith } = renderItem()
    fireEvent.error(thumbnail(container)!)

    rerenderWith({ title: 'Renamed' })

    expect(thumbnail(container)).toBeNull()
  })
})
