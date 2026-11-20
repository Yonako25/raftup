# Site officiel de raftup-games.com (nginx, fichiers statiques).
# Contexte de construction : la racine du dépôt (logo et polices viennent du launcher).
FROM nginxinc/nginx-unprivileged:1.27-alpine

COPY deploy/site/nginx.conf /etc/nginx/conf.d/default.conf
COPY deploy/site/www/ /usr/share/nginx/html/
COPY crates/launcher-ui/icons/logo-texte.png crates/launcher-ui/icons/logo.png crates/launcher-ui/icons/icon.png /usr/share/nginx/html/
COPY crates/launcher-ui/fonts/ /usr/share/nginx/html/fonts/

EXPOSE 8080
