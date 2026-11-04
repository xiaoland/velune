function fitDiagram() {
  const svg = document.querySelector('#diagram svg'), box = svg?.viewBox.baseVal;
  if (!box || box.width <= 0 || box.height <= 0) return;
  const fit = Math.min(1, (innerWidth - 48) / box.width, (innerHeight - 48) / box.height);
  svg.style.width = Math.max(1, box.width * fit) + 'px';
  svg.style.height = Math.max(1, box.height * fit) + 'px';
}
addEventListener('resize', fitDiagram);
window.renderArtifact = async function (source) {
  const dark = matchMedia('(prefers-color-scheme: dark)').matches;
  const diagram = document.getElementById('diagram'), error = document.getElementById('error');
  try {
  mermaid.initialize({ startOnLoad: false, securityLevel: 'strict', theme: dark ? 'dark' : 'default',
    suppressErrorRendering: true, flowchart: { htmlLabels: false },
    secure: ['secure', 'securityLevel', 'startOnLoad', 'maxTextSize', 'maxEdges', 'suppressErrorRendering'] });
    const result = await mermaid.render('artifact-diagram', source);
    diagram.innerHTML = result.svg;
    fitDiagram();
    // No Mermaid interactions, links or externally loaded resources.
    for (const node of diagram.querySelectorAll('script,iframe,object,embed,img,image')) node.remove();
    for (const node of diagram.querySelectorAll('a')) node.replaceWith(...node.childNodes);
    diagram.hidden = false; error.hidden = true;
  } catch (failure) {
    diagram.replaceChildren(); diagram.hidden = true; error.hidden = false;
    error.textContent = '无法渲染图表\n\n' + String(failure.message || failure);
  }
};
