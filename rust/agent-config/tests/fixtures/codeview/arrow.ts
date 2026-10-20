const add = (a: number, b: number) => a + b;

export const fetchAll = async (
  ids: string[],
  opts: Options = {},
): Promise<Item[]> => {
  return ids.map((id) => {
    return load(id, opts);
  });
};

const curried = (a: number) =>
  (b: number) =>
    a + b;

export const Card = ({ id, name }: Props = {}): string => {
  const label = (s: string) => s.trim();
  return `${id}:${label(name)}`;
};

const noSemi = (n: number) => {
  return n * 2;
}

const tail = 1;
