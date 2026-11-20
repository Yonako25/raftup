// Rafthub : bouton « Copier » (seul script de la console, ADR 0036).
// Un bouton `data-copy="<id>"` copie la valeur du champ `<id>` dans le presse-papiers.
// Sans JavaScript, le champ reste sélectionnable à la main.
document.querySelectorAll("[data-copy]").forEach(function (button) {
  button.addEventListener("click", function () {
    var field = document.getElementById(button.getAttribute("data-copy"));
    if (!field) { return; }
    var done = function () { button.textContent = "Copié"; };
    if (navigator.clipboard && navigator.clipboard.writeText) {
      navigator.clipboard.writeText(field.value).then(done, function () { field.select(); });
    } else {
      field.select();
    }
  });
});
