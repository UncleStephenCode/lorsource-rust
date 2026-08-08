FROM maven:3.9.14-eclipse-temurin-25

RUN apt-get update \
    && apt-get install -y --no-install-recommends postgresql-client gzip \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /opt/lorsource-java-db
COPY compat/java-db/pom.xml ./pom.xml
RUN mvn --batch-mode --no-transfer-progress --file pom.xml dependency:go-offline
COPY compat/java-db/ ./
RUN ./check-vendor.sh

ENTRYPOINT ["./manage.sh"]
CMD ["validate"]
