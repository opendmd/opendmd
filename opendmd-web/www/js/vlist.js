// A virtualised list: thousands of rows (3,781 in gifs/Consoles, 10,133 in
// filelist.txt) with only the visible ones in the DOM. Fixed row height; a
// pool of row elements is re-rendered on scroll.

import { h } from './util.js';

export class VList {
  constructor({ rowHeight = 40, render, onClick }) {
    this.rh = rowHeight;
    this.render = render;
    this.items = [];
    this.inner = h('div');
    this.el = h('div.vlist', { onscroll: () => this.draw() }, this.inner);
    this.pool = [];
    this.el.addEventListener('click', (e) => {
      const row = e.target.closest('.vrow');
      if (row && onClick) onClick(this.items[row._i], row._i, e);
    });
    new ResizeObserver(() => this.draw()).observe(this.el);
  }

  set(items) {
    this.items = items;
    this.inner.style.height = items.length * this.rh + 'px';
    this.draw(true);
  }

  refresh() { this.draw(true); }

  scrollTo(i) {
    const top = i * this.rh;
    if (top < this.el.scrollTop || top > this.el.scrollTop + this.el.clientHeight - this.rh) this.el.scrollTop = top - this.rh * 2;
  }

  draw(force) {
    const first = Math.max(0, Math.floor(this.el.scrollTop / this.rh) - 4);
    const count = Math.ceil((this.el.clientHeight || 600) / this.rh) + 8;
    const last = Math.min(this.items.length, first + count);
    if (!force && first === this.first && last === this.last) return;
    this.first = first; this.last = last;
    while (this.pool.length < last - first) {
      const r = h('div.vrow', { style: `height:${this.rh}px` });
      this.pool.push(r);
      this.inner.append(r);
    }
    this.pool.forEach((r, k) => {
      const i = first + k;
      if (i >= last) { r.hidden = true; return; }
      r.hidden = false;
      r._i = i;
      r.style.transform = `translateY(${i * this.rh}px)`;
      this.render(r, this.items[i], i);
    });
  }
}
