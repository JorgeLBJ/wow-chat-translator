// Cheap local check to avoid sending obviously Spanish messages to the translator.
// It only needs to be right when it says "Spanish"; anything else goes to the translator,
// which also reports the detected source language.

const ES = new Set(
  ("de la que el en y los se del las un una por con no su para es al lo como mas más pero sus le ya " +
    "este esta esto eso esa porque cuando muy sin sobre tambien también me hasta hay donde dónde quien " +
    "quién desde todo todos nos les ni otro otra yo tu tú él ella nosotros algo alguien alguno alguna " +
    "necesito necesita gracias hola vamos quiero tengo tiene estoy estas estás bueno buena aqui aquí " +
    "ahora ayuda ayudar puedo puede vendo compro busco grupo mazmorra tanque sanador favor")
    .split(" "),
);
const EN = new Set(
  ("the a an is are you i to and of in for on it this that with need anyone any what where who how " +
    "want can have has do does my your we they lol pls please thanks ty lf lfm lfg wts wtb")
    .split(" "),
);

export function looksSpanish(text) {
  if (/[ñ¿¡]/i.test(text)) return true;
  const words = text.toLowerCase().match(/\p{L}+/gu) ?? [];
  let es = 0;
  let en = 0;
  for (const w of words) {
    if (ES.has(w)) es++;
    if (EN.has(w)) en++;
  }
  return es >= 2 && es > en;
}
