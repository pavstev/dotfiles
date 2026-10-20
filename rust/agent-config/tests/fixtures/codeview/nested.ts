export function outer(a: number): number {
  const x = a + 1;
  if (a > 0) {
    x + 1;
  }
  function inner(b: number): number {
    function innermost() {
      return b;
    }
    return innermost() + x;
  }
  return inner(a);
}

function sibling() {
  return 1;
}

class Box {
  value = 1;
  constructor(v: number) {
    this.value = v;
  }
  static make(): Box {
    return new Box(0);
  }
  async load(
    id: string,
  ): Promise<void> {
    if (id) {
      await go(id);
    }
  }
}

interface Shape {
  area(): number;
}

type Pair = {
  a: number;
  b: number;
};

type Id = string;

enum Color {
  Red,
  Green,
}
