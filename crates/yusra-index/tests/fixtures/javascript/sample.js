const fs = require("fs");
import { join } from "./path.js";

/** Tambah dua angka. */
function add(a, b) {
  return a + b;
}

class Counter {
  count = 0;

  increment() {
    this.count = add(this.count, 1);
  }
}

module.exports = { add, Counter };
