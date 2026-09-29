import { describe, expect, it, vi } from 'vitest'
import { fireEvent, render, screen } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import { AddFeedModal } from './AddFeedModal'

function renderModal(
  onAdd: (url: string) => Promise<void> = vi.fn().mockResolvedValue(undefined),
  onClose: () => void = vi.fn(),
) {
  render(<AddFeedModal onAdd={onAdd} onClose={onClose} />)
  return { onAdd, onClose }
}

describe('AddFeedModal', () => {
  // 捕まえる変異: 07-modal-empty-guard (空送信ガードの削除)。
  // type="url" の入力は前後の空白を自ら落とすため、空白だけの状態は
  // DOM 経由では到達できない。ここでは到達できる空状態だけを確かめる
  it('does not call onAdd when submitted empty', async () => {
    const user = userEvent.setup()
    const { onAdd, onClose } = renderModal()
    const input = screen.getByPlaceholderText('https://example.com/feed.xml')
    const form = input.closest('form')
    if (!form) throw new Error('missing form')

    fireEvent.submit(form)
    await user.type(input, 'https://example.com/feed.xml')
    await user.clear(input)
    fireEvent.submit(form)

    expect(onAdd).not.toHaveBeenCalled()
    expect(onClose).not.toHaveBeenCalled()
  })

  // 捕まえる変異: 06-modal-submit (onAdd 呼び出しの削除)
  it('calls onAdd with the typed URL on submit', async () => {
    const user = userEvent.setup()
    const onAdd = vi.fn().mockResolvedValue(undefined)
    renderModal(onAdd)

    await user.type(
      screen.getByPlaceholderText('https://example.com/feed.xml'),
      'https://example.com/feed.xml',
    )
    await user.click(screen.getByText('追加'))
    expect(onAdd).toHaveBeenCalledTimes(1)
    expect(onAdd).toHaveBeenCalledWith('https://example.com/feed.xml')
  })

  // 捕まえる変異: 10-modal-loading-display。解決を遅延させ、応答待ちの間の表示を確定させる
  it('shows the loading state while onAdd is pending', async () => {
    const user = userEvent.setup()
    let resolve!: () => void
    const gate = new Promise<void>((r) => {
      resolve = r
    })
    renderModal(vi.fn().mockReturnValue(gate))

    await user.type(
      screen.getByPlaceholderText('https://example.com/feed.xml'),
      'https://example.com/feed.xml',
    )
    const click = user.click(screen.getByText('追加'))
    expect(await screen.findByText('追加中...')).not.toBeNull()
    expect(screen.getByText('追加中...').closest('button')?.disabled).toBe(true)
    resolve()
    await click
  })

  // 捕まえる変異: 09-modal-loading。送信が終われば待機表示に戻る（setLoading(false) の削除で落ちる）
  it('returns the button to its idle label after onAdd resolves', async () => {
    const user = userEvent.setup()
    renderModal()

    await user.type(
      screen.getByPlaceholderText('https://example.com/feed.xml'),
      'https://example.com/feed.xml',
    )
    await user.click(screen.getByText('追加'))
    expect(await screen.findByText('追加')).not.toBeNull()
  })

  // 捕まえる変異: 08-modal-cancel (キャンセルボタンの onClose 切り離し)
  it('calls onClose from the cancel button', async () => {
    const user = userEvent.setup()
    const onAdd = vi.fn().mockResolvedValue(undefined)
    const onClose = vi.fn()
    renderModal(onAdd, onClose)

    await user.click(screen.getByText('キャンセル'))
    expect(onClose).toHaveBeenCalledTimes(1)
    expect(onAdd).not.toHaveBeenCalled()
  })

  // 捕まえる変異: 11-modal-overlay (overlay の onClose 切り離し)
  it('calls onClose from the overlay but not from inside the dialog', async () => {
    const user = userEvent.setup()
    const onClose = vi.fn()
    renderModal(vi.fn().mockResolvedValue(undefined), onClose)
    const overlay = document.querySelector('.modal-overlay')
    if (!overlay) throw new Error('missing .modal-overlay')
    const dialog = document.querySelector('.modal')
    if (!dialog) throw new Error('missing .modal')

    fireEvent.click(dialog)
    expect(onClose).not.toHaveBeenCalled()
    await user.click(overlay)
    expect(onClose).toHaveBeenCalledTimes(1)
  })
})
