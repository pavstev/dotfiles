function a() {
  const s = "}";
  const t = '{';
  return s + t;
}

function b(name: string, flag: boolean) {
  const msg = `open { ${flag ? `nested } ${name}` : "}"} close`;
  return msg;
}

function c(input: string) {
  const re = /[{}]+\/}/g;
  const half = input.replace(/\{/, "");
  return re.test(half) ? 1 / 2 : 4 / 2;
}

// function fake() {
/* class Nope {
   function alsoNope() {
*/
function d() {
  // }
  /* } */
  return "ok"; // it's { open
}

function e() {
  const q = "say \"}\" now";
  const r = 'it\'s {';
  return q + r;
}
