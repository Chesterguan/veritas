# VERITAS — Environnement d'Exécution d'Agents de Confiance

**Livre blanc v0.3**

*Léger · Déterministe · Auditable · Vérifiable*

> Dernière mise à jour : 2026-02-18

---

## Table des matières

1. [Vision](#1-vision)
2. [Motivation](#2-motivation)
3. [Positionnement](#3-positioning)
4. [Philosophie de conception](#4-design-philosophy)
5. [Lignée ZeroClaw et OpenClaw](#5-zeroclaw-and-openclaw-lineage)
6. [Modèle système](#6-system-model)
7. [Modèle de confiance](#7-trust-model)
8. [Modèle de capacités](#8-capability-model)
9. [Politique et gouvernance](#9-policy-and-governance)
10. [Audit et traçabilité](#10-audit-and-traceability)
11. [Modèle de vérification](#11-verification-model)
12. [Modèle de sécurité](#12-security-model)
13. [Indépendance du modèle de données](#13-data-model-independence)
14. [Domaine de référence : Santé](#14-reference-domain-healthcare)
15. [Panorama et différenciation](#15-landscape-and-differentiation)
16. [Extensibilité](#16-extensibility)

---

## 1. Vision

VERITAS est un environnement d'exécution léger, déterministe, lié à des politiques, auditable et vérifiable, destiné aux agents IA opérant dans des environnements réglementés. Le système privilégie la confiance, le contrôle et les preuves sur l'autonomie et l'intelligence opaque.

VERITAS est conçu comme une couche d'exécution fondatrice — ni une application, ni un produit d'automatisation, ni une plateforme de gouvernance lourde. Il rend les environnements d'exécution d'agents existants dignes de confiance sans les ralentir.

## 2. Motivation

Les agents IA modernes sont puissants mais fondamentalement peu fiables dans les environnements réglementés. Leur comportement est souvent non déterministe, non auditable et difficile à vérifier. Des frameworks comme OpenClaw et ZeroClaw ont prouvé que les agents peuvent être rapides, légers et déployables partout — mais ils n'ont pas été conçus pour des environnements où chaque action doit être traçable, contrainte par des politiques et vérifiable.

Dans le même temps, les solutions de gouvernance d'entreprise abordent le problème depuis la direction opposée : elles ajoutent des intergiciels lourds, des moteurs de règles complexes et des pipelines de validation lents qui détruisent la vitesse et la simplicité qui rendaient les agents légers utiles en premier lieu.

VERITAS emprunte une voie différente. Au lieu de reconstruire les agents de zéro ou de les envelopper dans de la bureaucratie, VERITAS fournit une fine couche d'exécution de confiance qui améliore les bons agents — sûrs, auditables et vérifiables — tout en préservant la nature légère, rapide et composable des environnements d'exécution sur lesquels ils fonctionnent déjà.

## 3. Positionnement

### Ce qu'est VERITAS

- Un environnement d'exécution d'agents de confiance
- Une couche légère d'application des politiques et d'audit
- Une frontière de confiance entre les agents et le monde réel
- Un fondement pour le déploiement d'agents IA dans des contextes réglementés

### Ce que VERITAS N'EST PAS

- Pas un assistant IA ni un chatbot
- Pas une plateforme d'automatisation
- Pas un système de santé ni un outil clinique
- Pas une plateforme de données ni un pipeline ETL
- Pas un intergiciel de gouvernance lourd
- Pas un remplacement des frameworks d'agents — il les enveloppe

### L'analogie Red Hat

Le noyau Linux est rapide, minimal et fonctionne partout. Red Hat ne l'a pas remplacé ni ralenti — il l'a rendu prêt pour l'entreprise en ajoutant confiance, support, certification et gouvernance autour d'un fondement éprouvé.

VERITAS suit le même modèle :

```
Linux Kernel        →  ZeroClaw / OpenClaw    (fast, minimal, runs anywhere)
Red Hat Enterprise  →  VERITAS                (trusted, governed, auditable)
```

Les développeurs d'agents ne doivent pas ressentir le poids de la gouvernance. Construire sur VERITAS doit sembler comme construire sur ZeroClaw — rapide, simple, composable — avec la confiance appliquée de manière transparente par l'environnement d'exécution, et non par le code applicatif.

## 4. Philosophie de conception

Les dix principes qui régissent toutes les décisions de conception de VERITAS :

1. **Contrôle plutôt qu'autonomie** — le système, pas le LLM, décide de ce qui se passe
2. **Preuves plutôt qu'intelligence** — une sortie prouvable vaut mieux qu'un raisonnement habile
3. **Déterminisme plutôt qu'émergence** — comportement prévisible, toujours
4. **Refus par défaut** — rien ne s'exécute sans être explicitement autorisé
5. **Sécurité basée sur les capacités** — fine, déclarative, composable
6. **Base de calcul de confiance minimale** — petite surface, moins de bogues, plus facile à auditer
7. **Auditabilité par conception** — pas ajoutée après coup, intégrée dès le premier jour
8. **Exécution vérifiable** — chaque sortie peut être validée indépendamment
9. **Intervention humaine toujours possible** — les machines proposent, les humains disposent
10. **Indépendance du modèle de données** — pas de couplage avec FHIR, OMOP ou un schéma de domaine quelconque

Et un méta-principe au-dessus de tout :

> **Légèreté par conviction.** La gouvernance ne doit pas être la raison pour laquelle les agents deviennent lents, lourds ou difficiles à construire. Si VERITAS dégrade l'expérience développeur, VERITAS a échoué.

## 5. Lignée ZeroClaw et OpenClaw

VERITAS hérite de sa philosophie d'exécution de deux projets open source qui ont prouvé que les agents peuvent être rapides, petits et pratiques :

### ZeroClaw

ZeroClaw est un environnement d'exécution d'agents IA ultra-léger écrit en Rust. Il démarre en moins de 10 ms, se distribue sous forme de binaire de ~3,4 Mo, fonctionne sur ARM/x86/RISC-V, et se déploie sur du matériel coûtant moins de 10 $. ZeroClaw met l'accent sur un noyau d'agent minimal, un flux d'exécution explicite, une composabilité par traits, zéro dépendance externe et une petite base de calcul de confiance.

### OpenClaw

OpenClaw a porté le modèle d'agent-comme-assistant-personnel au grand public — persistant, toujours disponible, multi-canal (WhatsApp, Slack, Telegram, etc.), auto-hébergé et extensible via plus de 100 AgentSkills. OpenClaw a prouvé que les agents IA ne sont pas des jouets ni des démonstrations ; ils sont de l'infrastructure.

### Ce que VERITAS hérite

De ZeroClaw :
- Conception de noyau minimal
- Flux d'exécution explicite (pas d'état caché)
- Architecture composable par traits
- Petite base de calcul de confiance
- Portabilité et faibles besoins en ressources

De OpenClaw :
- État d'esprit agent-comme-infrastructure
- Extensibilité via compétences/capacités
- Schémas de déploiement réels
- Modèle d'opération multi-canal, toujours disponible

### Ce que VERITAS ajoute

Ce que ni ZeroClaw ni OpenClaw n'ont été conçus pour fournir — et ce que les environnements réglementés exigent :

- **Moteur de politique à refus par défaut** — chaque action vérifiée par politique avant exécution
- **Piste d'audit immuable** — flux d'événements en ajout seul, inviolable, rejouable
- **Vérification des sorties** — validation de schéma, de règles et de risques avant livraison
- **Frontière de confiance formelle** — LLM, outils et données traités comme non fiables par architecture
- **Sécurité au niveau des capacités** — permissions et déclarations d'effets secondaires par outil
- **Points d'intervention humaine** — flux d'approbation pour les opérations sensibles

VERITAS ne bifurque pas et ne remplace pas ZeroClaw. Il ajoute la confiance par-dessus le même fondement léger.

## 6. Modèle système

L'exécution d'agents dans VERITAS est modélisée comme une machine à états déterministe opérant sur des capacités contrôlées.

### Boucle d'exécution

```
State → Policy → Capability → Audit → Verify → Next State
```

Chaque transition est explicite, vérifiée par politique, auditée et validée avant que l'agent passe à l'état suivant. La boucle est intentionnellement minimale — pas d'intergiciel caché, pas d'orchestration lourde, pas de couches d'abstraction ajoutant de la latence.

### Propriétés de la machine à états

- **Déterministe** — même entrée + même politique = même chemin d'exécution
- **Observable** — chaque transition d'état est un événement d'audit
- **Interruptible** — l'intervention humaine peut stopper ou rediriger à n'importe quelle transition
- **Rejouable** — les traces d'exécution peuvent reproduire tout run passé

## 7. Modèle de confiance

La confiance dans VERITAS est dérivée de l'exécution déterministe, des pistes d'audit immuables, des décisions de politique explicites et des sorties vérifiables.

Le système ne fait pas confiance inheremment au raisonnement des LLM, aux outils externes, aux données d'entrée ou aux environnements d'exécution. Ce n'est pas de la paranoïa — c'est de l'honnêteté architecturale. Les LLM hallucinent, les outils ont des bogues, les données peuvent être empoisonnées, et les environnements peuvent être compromis.

### Frontière de confiance

| De confiance | Non fiable |
|---------|-----------|
| Noyau de l'environnement d'exécution | LLM |
| Moteur de politique | Outils |
| Moteur d'audit | Données d'entrée |
| Vérificateur | Environnement externe |

La confiance n'est pas supposée. Elle est dérivée de preuves — pistes d'audit, journaux de politique et résultats de vérification.

## 8. Modèle de capacités

Les capacités représentent des outils contraints avec des schémas explicites, des permissions et des déclarations d'effets secondaires.

Toutes les interactions avec le monde extérieur doivent se faire via des capacités sous contrôle de politique. Une capacité n'est pas simplement un appel de fonction — c'est un contrat qui déclare :

- Ce qu'elle fait (schéma)
- Ce qu'elle requiert (permissions)
- Ce qu'elle modifie (effets secondaires)
- Les risques qu'elle comporte (niveau de risque)

Il s'agit de sécurité basée sur les capacités : les agents ne peuvent accéder à rien qui n'ait pas été explicitement accordé comme capacité, et chaque invocation de capacité est vérifiée par politique et auditée.

## 9. Politique et gouvernance

VERITAS applique un refus par défaut à l'exécution. Les décisions de politique évaluent le sujet, l'action, la ressource et le contexte pour déterminer l'un des trois résultats possibles :

- **Autoriser** — l'action se poursuit, événement d'audit enregistré
- **Refuser** — l'action est bloquée, événement d'audit enregistré avec la raison
- **Requérir une approbation** — l'action est suspendue en attente d'une révision humaine

La politique est déterministe, explicable et auditable. Chaque décision de politique peut être retracée jusqu'à une règle spécifique, et chaque règle peut être inspectée par un auditeur humain.

### Légèreté par conception

Le moteur de politique n'est pas une plateforme de règles métier lourde. C'est un évaluateur rapide et déterministe — plus proche d'un pare-feu que d'un moteur de flux de travail d'entreprise. L'évaluation de politique doit ajouter des microsecondes, pas des millisecondes.

## 10. Audit et traçabilité

Tous les événements d'exécution sont enregistrés dans un flux d'événements en ajout seul formant un graphe d'exécution vérifiable. Chaque événement contient :

- Les transitions d'état
- Les appels de capacités
- Les décisions de politique
- Les résultats de vérification
- Les horodatages et l'ordre causal

Le système prend en charge des traces d'exécution rejouables et inviolables. Toute exécution peut être reproduite et vérifiée indépendamment après coup.

### Pourquoi c'est important

Dans les environnements réglementés, « ça a fonctionné » ne suffit pas. Vous devez prouver *comment* ça a fonctionné, *pourquoi* chaque décision a été prise, et *ce qui* a été vérifié. La piste d'audit n'est pas un fichier journal — c'est la preuve que le système s'est comporté correctement.

## 11. Modèle de vérification

VERITAS applique le principe du moindre privilège, l'exécution isolée des capacités et un contrôle strict des frontières.

L'environnement d'exécution ne permet pas l'accès direct au système et suppose que tous les composants externes ne sont pas fiables. La sécurité n'est pas une fonctionnalité ajoutée par-dessus — c'est une conséquence de l'architecture :

- Politique de refus par défaut → aucune action non autorisée
- Accès basé sur les capacités → pas d'autorité ambiante
- Piste d'audit immuable → aucune falsification non détectée
- Vérification des sorties → aucun livrable non validé
- Frontière de confiance → pas de confiance implicite dans le LLM ou les outils

## 12. Modèle de sécurité

VERITAS applique le principe du moindre privilège, l'exécution isolée des capacités et un contrôle strict des frontières.

L'environnement d'exécution ne permet pas l'accès direct au système et suppose que tous les composants externes ne sont pas fiables. La sécurité n'est pas une fonctionnalité ajoutée par-dessus — c'est une conséquence de l'architecture :

- Politique de refus par défaut → aucune action non autorisée
- Accès basé sur les capacités → pas d'autorité ambiante
- Piste d'audit immuable → aucune falsification non détectée
- Vérification des sorties → aucun livrable non validé
- Frontière de confiance → pas de confiance implicite dans le LLM ou les outils

## 13. Indépendance du modèle de données

Le noyau de l'environnement d'exécution VERITAS est indépendant des modèles de données spécifiques à la santé ou à l'entreprise tels que FHIR, OMOP, HL7 ou des schémas propriétaires.

Les adaptateurs spécifiques au domaine sont implémentés en externe via des capacités. Cela signifie que VERITAS peut gouverner des agents opérant dans la santé, la finance, le droit ou tout autre domaine réglementé — sans que le noyau de l'environnement d'exécution connaisse ou se soucie du modèle de données du domaine.

Le noyau parle capacités, politiques et événements d'audit. Le domaine parle ce dont il a besoin — via des adaptateurs qui sont eux-mêmes des capacités, soumis aux mêmes contrôles de politique et d'audit que tout le reste.

## 14. Domaine de référence : Santé

Bien que VERITAS soit conçu comme indépendant du domaine, son implémentation de référence cible la **santé** — l'un des environnements les plus réglementés et les plus critiques pour le déploiement d'agents IA.

### Pourquoi la santé

La santé est le domaine où les conséquences d'agents IA non contrôlés sont les plus graves :

- Une vérification incorrecte d'interaction médicamenteuse peut nuire à un patient
- Un accès non autorisé aux données peut violer HIPAA/GDPR
- Une décision clinique non auditée ne peut pas être défendue devant un tribunal
- Une sortie non déterministe ne peut pas être reproduite pour révision

Si VERITAS peut gagner la confiance dans la santé, il peut la gagner partout.

### Défis spécifiques à la santé que VERITAS adresse

| Défi | Réponse de VERITAS |
|---|---|
| Sensibilité des données patient | Accès basé sur les capacités — les agents ne voient que ce que la politique autorise |
| Conformité réglementaire (HIPAA, GDPR, MDR) | Piste d'audit immuable prouvant chaque accès et décision |
| Aide à la décision clinique | Vérification des sorties — chaque recommandation validée avant livraison |
| Interopérabilité (FHIR, HL7, OMOP) | Adaptateurs de domaine comme capacités — le noyau reste propre |
| Exigences de supervision humaine | Politique de requête d'approbation — révision du clinicien pour les opérations sensibles |
| Reproductibilité pour les audits | Traces d'exécution rejouables — recréer tout run d'agent passé |

### Ce que VERITAS ne fait PAS dans la santé

- N'interprète pas les données cliniques
- Ne prend pas de décisions cliniques
- Ne remplace pas le jugement du clinicien
- Ne stocke pas ni ne gère les dossiers patients
- N'implémente pas FHIR/HL7 — les adaptateurs le font

VERITAS gouverne l'**agent** qui fait ces choses. Il garantit que l'agent opère dans le cadre de la politique, produit des preuves auditables et livre des sorties vérifiables — indépendamment du système clinique auquel il se connecte.

## 15. Panorama et différenciation

### Frameworks d'agents (Construire des agents)

Des frameworks comme LangGraph, CrewAI, AutoGen et OpenClaw aident les développeurs à construire des agents. Ils se concentrent sur l'orchestration, l'utilisation d'outils et la coordination multi-agents. Ce ne sont pas des concurrents — ce sont des **consommateurs** potentiels de VERITAS. Un agent construit avec n'importe quel framework peut s'exécuter à l'intérieur de l'environnement d'exécution VERITAS.

### Systèmes de garde-fous (Filtrer les E/S)

Des systèmes comme Guardrails AI, LlamaFirewall et Superagent valident les entrées et les sorties. Ils détectent les injections de prompt, les contenus dangereux et les violations de schéma. Ils sont utiles mais incomplets — ils filtrent à la frontière sans contrôler l'exécution elle-même.

### Gouvernance d'entreprise (Documents de politique)

Des frameworks comme NIST AI RMF, le Modèle de gouvernance IA de Singapour et AAGATE fournissent des principes de gouvernance et des méthodologies d'évaluation. Ils décrivent *ce qui* doit se passer. VERITAS implémente *comment* cela se passe à l'exécution.

### Où se situe VERITAS

```
┌─────────────────────────────────────────────────────┐
│              Application / Agent Code               │
│         (LangGraph, CrewAI, OpenClaw, etc.)         │
├─────────────────────────────────────────────────────┤
│                    VERITAS                           │
│   Policy Engine │ Audit Trail │ Verifier │ Caps     │
├─────────────────────────────────────────────────────┤
│              Agent Runtime Kernel                    │
│            (ZeroClaw or equivalent)                  │
└─────────────────────────────────────────────────────┘
```

VERITAS est la **couche intermédiaire** — sous l'application, au-dessus du noyau. Il ajoute la confiance sans ajouter de poids.

## 16. Extensibilité

VERITAS fournit des interfaces standardisées pour :

- Les capacités (outils et adaptateurs spécifiques au domaine)
- Les moteurs de politique (ensembles de règles personnalisés par environnement)
- Le stockage d'audit (backends enfichables — local, cloud, blockchain)
- Les modules de vérification (validateurs personnalisés par domaine)

Les contributeurs externes peuvent étendre le système sans modifier le noyau de confiance. Le modèle d'extension suit le même principe que l'architecture par traits de ZeroClaw : composable, interchangeable et minimal.

---

*Fin du livre blanc VERITAS v0.3*
